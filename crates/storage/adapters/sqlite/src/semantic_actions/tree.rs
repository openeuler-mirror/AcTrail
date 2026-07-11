//! Query helpers for semantic action tree views.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use rusqlite::types::Value;
use rusqlite::{Connection, Params, Row, params, params_from_iter};
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionStoreError,
    attr_keys as attrs,
};

use crate::SqliteStorage;
use crate::records::decode_map;
use crate::semantic_actions::action_ids::resolve_action_key;
use crate::semantic_actions::codebook::sqlite::{
    action_kind_code, action_kind_code_from_str, decode_link_confidence, decode_link_role,
    link_role_code_from_str,
};
use crate::semantic_actions::cold_fields::decode_text_from_row_with_prefix;
use crate::semantic_actions::store::{
    ACTION_SELECT_COLUMNS, action_cold_field_join, action_from_row, link_cold_field_join,
    read_evidence,
};
use crate::semantic_actions::tree_metadata::{
    child_count_for_parent, effective_incoming_link_absence_predicate, effective_link_value_count,
    invalidated_action_attrs, invalidated_link_attrs, load_child_metadata,
    push_effective_link_values,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionChildRow {
    pub link: SemanticActionLink,
    pub action: SemanticAction,
    pub child_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionChildPage {
    pub rows: Vec<SemanticActionChildRow>,
    pub total_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticActionChildPageQuery {
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticActionSummary {
    pub actions: usize,
    pub links: usize,
    pub roots: usize,
}

impl SqliteStorage {
    pub fn semantic_action_summary(
        &self,
        trace_id: TraceId,
    ) -> Result<SemanticActionSummary, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        let actions = count_rows(
            &connection,
            "SELECT COUNT(*) FROM semantic_actions WHERE trace_id = ?1",
            params![trace_id.get()],
            "count_semantic_actions",
        )?;
        let links = count_rows(
            &connection,
            "SELECT COUNT(*) FROM semantic_action_links WHERE trace_id = ?1",
            params![trace_id.get()],
            "count_semantic_action_links",
        )?;
        let root_predicate = effective_incoming_link_absence_predicate("action");
        let root_query = format!(
            "SELECT COUNT(*) FROM semantic_actions action
             WHERE action.trace_id = ?
             {root_predicate}"
        );
        let mut root_values = Vec::with_capacity(1 + effective_link_value_count(None));
        root_values.push(trace_id_value(trace_id)?);
        push_effective_link_values(&mut root_values, None)?;
        let roots = count_rows(
            &connection,
            &root_query,
            params_from_iter(root_values),
            "count_semantic_action_roots",
        )?;
        Ok(SemanticActionSummary {
            actions,
            links,
            roots,
        })
    }

    pub fn observed_agent_semantic_action(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<SemanticAction>, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        let action_cold_join = action_cold_field_join();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ACTION_SELECT_COLUMNS}
                     FROM semantic_actions action
                     JOIN semantic_action_ids ids
                       ON ids.action_key = action.action_key
                     {action_cold_join}
                     WHERE action.trace_id = ?1
                     AND action.kind_code = ?2
                     AND action.agent_observed = 1
                     ORDER BY action.start_time ASC, ids.action_id ASC"
            ))
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_observed_agent_action", error.to_string())
            })?;
        let rows = statement
            .query_map(
                params![
                    trace_id.get(),
                    action_kind_code(SemanticActionKind::ProcessExec),
                ],
                action_from_row,
            )
            .map_err(|error| {
                SemanticActionStoreError::new("query_observed_agent_action", error.to_string())
            })?;
        for row in rows {
            let mut action = row.map_err(|error| {
                SemanticActionStoreError::new("map_observed_agent_action", error.to_string())
            })?;
            if action
                .attributes
                .get(attrs::agent::IDENTITY_STATUS)
                .is_some_and(|status| status == "observed")
                && !invalidated_action_attrs(&action.attributes)
            {
                action.evidence = read_evidence(&connection, &action.action_id)?;
                return Ok(Some(action));
            }
        }
        Ok(None)
    }

    pub fn semantic_action_children(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, SemanticActionStoreError> {
        self.semantic_action_children_with_kind_filter(
            trace_id,
            parent_action_id,
            roles,
            child_roles,
            &[],
        )
    }

    pub fn semantic_action_children_page(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionChildPage, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let total_count = {
            let connection = self.connection().borrow();
            child_count_for_parent(&connection, trace_id, parent_action_id, roles)?
        };
        let mut rows = self.semantic_action_children_with_kind_filter_page(
            trace_id,
            parent_action_id,
            roles,
            &[],
            Some(page),
        )?;
        let connection = self.connection().borrow();
        load_child_metadata(&connection, trace_id, &mut rows, child_roles)?;
        Ok(SemanticActionChildPage { rows, total_count })
    }

    pub fn semantic_action_children_matching_kinds(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        child_kinds: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, SemanticActionStoreError> {
        if child_kinds.is_empty() {
            return Ok(Vec::new());
        }
        self.semantic_action_children_with_kind_filter(
            trace_id,
            parent_action_id,
            roles,
            child_roles,
            child_kinds,
        )
    }

    pub fn semantic_action_for_process_kind(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        kind: &str,
    ) -> Result<Option<SemanticAction>, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        let action_cold_join = action_cold_field_join();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ACTION_SELECT_COLUMNS}
                     FROM semantic_actions action
                     JOIN semantic_action_ids ids
                       ON ids.action_key = action.action_key
                     {action_cold_join}
                     WHERE action.trace_id = ?1
                       AND action.kind_code = ?2
                       AND action.process_id = ?3
                     ORDER BY action.start_time ASC, ids.action_id ASC
                     LIMIT 1"
            ))
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "prepare_semantic_action_for_process_kind",
                    error.to_string(),
                )
            })?;
        let mut rows = statement
            .query(params![
                trace_id.get(),
                action_kind_code_from_str(kind)?,
                process.get(),
            ])
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "query_semantic_action_for_process_kind",
                    error.to_string(),
                )
            })?;
        let Some(row) = rows.next().map_err(|error| {
            SemanticActionStoreError::new(
                "step_semantic_action_for_process_kind",
                error.to_string(),
            )
        })?
        else {
            return Ok(None);
        };
        let mut action = action_from_row(row).map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_for_process_kind", error.to_string())
        })?;
        action.evidence = read_evidence(&connection, &action.action_id)?;
        Ok(Some(action))
    }

    fn semantic_action_children_with_kind_filter(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        child_kinds: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, SemanticActionStoreError> {
        let mut children = self.semantic_action_children_with_kind_filter_page(
            trace_id,
            parent_action_id,
            roles,
            child_kinds,
            None,
        )?;
        let connection = self.connection().borrow();
        load_child_metadata(&connection, trace_id, &mut children, child_roles)?;
        Ok(children)
    }

    fn semantic_action_children_with_kind_filter_page(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_kinds: &[&str],
        page: Option<SemanticActionChildPageQuery>,
    ) -> Result<Vec<SemanticActionChildRow>, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        if roles.is_empty() {
            return Ok(Vec::new());
        }
        let kind_filter = if child_kinds.is_empty() {
            String::new()
        } else {
            format!(
                " AND action.kind_code IN ({})",
                sql_placeholders(child_kinds.len())
            )
        };
        let connection = self.connection().borrow();
        let Some(parent_action_key) = resolve_action_key(&connection, parent_action_id)? else {
            return Ok(Vec::new());
        };
        let page_filter = page.map(|_| " LIMIT ? OFFSET ?").unwrap_or_default();
        let action_cold_join = action_cold_field_join();
        let link_cold_join = link_cold_field_join();
        let query = format!(
            "SELECT {ACTION_SELECT_COLUMNS},
                    parent_ids.action_id AS parent_action_id,
                    ids.action_id AS child_action_id,
                    link.role_code AS role_code,
                    link.confidence_code AS confidence_code,
                    link.valid AS valid,
                    link.attributes AS link_legacy_attributes,
                    link_attrs.encoding_code AS link_attributes_encoding_code,
                    link_attrs.uncompressed_bytes AS link_attributes_uncompressed_bytes,
                    link_attrs.value_hash AS link_attributes_value_hash,
                    link_attrs.payload AS link_attributes_payload
             FROM semantic_action_links link
             JOIN semantic_actions action
               ON action.action_key = link.child_action_key
             JOIN semantic_action_ids ids
               ON ids.action_key = action.action_key
             JOIN semantic_action_ids parent_ids
               ON parent_ids.action_key = link.parent_action_key
             {action_cold_join}
             {link_cold_join}
             WHERE link.trace_id = ?
               AND link.parent_action_key = ?
               AND link.link_valid_code = 1
               AND link.role_code IN ({})
               {}
             ORDER BY action.start_time ASC, ids.action_id ASC, link.role_code ASC{}",
            sql_placeholders(roles.len()),
            kind_filter,
            page_filter
        );
        let mut values =
            role_and_kind_query_values(trace_id, parent_action_key, roles, child_kinds)?;
        if let Some(page) = page {
            values.push(usize_value(page.limit, "semantic_action_child_limit")?);
            values.push(usize_value(page.offset, "semantic_action_child_offset")?);
        }
        let mut statement = connection.prepare(&query).map_err(|error| {
            SemanticActionStoreError::new("prepare_semantic_action_children", error.to_string())
        })?;
        let rows = statement
            .query_map(params_from_iter(values), child_row_from_row)
            .map_err(|error| {
                SemanticActionStoreError::new("query_semantic_action_children", error.to_string())
            })?;
        let mut children = Vec::new();
        let mut seen = BTreeSet::new();
        for row in rows {
            let child = row.map_err(|error| {
                SemanticActionStoreError::new("map_semantic_action_child", error.to_string())
            })?;
            if invalidated_action_attrs(&child.action.attributes)
                || !child.link.valid
                || invalidated_link_attrs(
                    &child.link.attributes,
                    child.link.role,
                    &child.action.attributes,
                )
                || !seen.insert(child.action.action_id.clone())
            {
                continue;
            }
            children.push(child);
        }
        Ok(children)
    }

    pub fn semantic_action_child_count(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
    ) -> Result<usize, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        child_count_for_parent(&connection, trace_id, parent_action_id, roles)
    }

    pub fn semantic_action_by_id(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Option<SemanticAction>, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        let action = crate::semantic_actions::store::read_action_by_id(&connection, action_id)?;
        Ok(action.filter(|action| action.trace_id == trace_id))
    }
}

fn usize_value(value: usize, stage: &'static str) -> Result<Value, SemanticActionStoreError> {
    i64::try_from(value)
        .map(Value::Integer)
        .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))
}

fn trace_id_value(trace_id: TraceId) -> Result<Value, SemanticActionStoreError> {
    i64::try_from(trace_id.get())
        .map(Value::Integer)
        .map_err(|error| {
            SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
        })
}

fn ensure_semantic_trace(
    storage: &SqliteStorage,
    trace_id: TraceId,
) -> Result<(), SemanticActionStoreError> {
    if storage.is_purged(trace_id) {
        return Err(SemanticActionStoreError::new(
            "read_semantic_actions",
            "trace has been purged",
        ));
    }
    Ok(())
}

fn count_rows<P: Params>(
    connection: &Connection,
    query: &str,
    params: P,
    stage: &str,
) -> Result<usize, SemanticActionStoreError> {
    let count = connection
        .query_row(query, params, |row| row.get::<_, i64>(0))
        .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))?;
    usize::try_from(count).map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))
}

fn child_row_from_row(row: &Row<'_>) -> Result<SemanticActionChildRow, rusqlite::Error> {
    let action = action_from_row(row)?;
    let link = SemanticActionLink {
        trace_id: action.trace_id,
        parent_action_id: row.get("parent_action_id")?,
        child_action_id: row.get("child_action_id")?,
        role: decode_link_role(row.get::<_, i64>("role_code")?)?,
        confidence: decode_link_confidence(row.get::<_, i64>("confidence_code")?)?,
        valid: row.get("valid")?,
        evidence: Vec::new(),
        attributes: decode_map(&decode_text_from_row_with_prefix(
            row,
            "link_legacy_attributes",
            "link_attributes",
        )?),
    };
    Ok(SemanticActionChildRow {
        action,
        link,
        child_count: 0,
    })
}

fn role_and_kind_query_values(
    trace_id: TraceId,
    parent_action_key: i64,
    roles: &[&str],
    child_kinds: &[&str],
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let trace_id = i64::try_from(trace_id.get()).map_err(|error| {
        SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
    })?;
    let mut values = Vec::with_capacity(2 + roles.len() + child_kinds.len());
    values.push(Value::Integer(trace_id));
    values.push(Value::Integer(parent_action_key));
    for role in roles {
        values.push(Value::Integer(i64::from(link_role_code_from_str(role)?)));
    }
    for kind in child_kinds {
        values.push(Value::Integer(i64::from(action_kind_code_from_str(kind)?)));
    }
    Ok(values)
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

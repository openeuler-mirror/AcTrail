//! Query helpers for semantic action tree views.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use rusqlite::types::Value;
use rusqlite::{Connection, Params, Row, params, params_from_iter};
use semantic_action::{SemanticAction, SemanticActionLink, SemanticActionStoreError};

use crate::SqliteStorage;
use crate::records::decode_map;
use crate::semantic_actions::store::{
    action_from_row, decode_link_confidence, decode_link_role, read_evidence,
};
use crate::semantic_actions::tree_metadata::{
    child_count_for_parent, invalidated_action_attrs, invalidated_link_attrs, load_child_metadata,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionChildRow {
    pub link: SemanticActionLink,
    pub action: SemanticAction,
    pub child_count: usize,
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
        let roots = count_rows(
            &connection,
            "SELECT COUNT(*) FROM semantic_actions action
             WHERE action.trace_id = ?1
             AND NOT EXISTS (
                SELECT 1 FROM semantic_action_links link
                WHERE link.trace_id = action.trace_id
                AND link.child_action_id = action.action_id
             )",
            params![trace_id.get()],
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
        let mut statement = connection
            .prepare(
                "SELECT * FROM semantic_actions
                 WHERE trace_id = ?1
                 AND kind = 'process.exec'
                 AND attributes LIKE ?2
                 ORDER BY start_time ASC, action_id ASC",
            )
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_observed_agent_action", error.to_string())
            })?;
        let rows = statement
            .query_map(
                params![trace_id.get(), "%agent.identity.status=observed%"],
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
                .get("agent.identity.status")
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
        let mut statement = connection
            .prepare(
                "SELECT * FROM semantic_actions
                 WHERE trace_id = ?1
                   AND kind = ?2
                   AND process_pid = ?3
                   AND process_task_id IS ?4
                   AND process_start_ticks = ?5
                   AND process_pid_namespace IS ?6
                   AND process_generation = ?7
                 ORDER BY start_time ASC, action_id ASC
                 LIMIT 1",
            )
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "prepare_semantic_action_for_process_kind",
                    error.to_string(),
                )
            })?;
        let mut rows = statement
            .query(params![
                trace_id.get(),
                kind,
                process.pid,
                process.task_id,
                process.start_time_ticks,
                process
                    .pid_namespace
                    .as_ref()
                    .map(|namespace| namespace.as_str().to_string()),
                process.generation,
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
        ensure_semantic_trace(self, trace_id)?;
        if roles.is_empty() {
            return Ok(Vec::new());
        }
        let kind_filter = if child_kinds.is_empty() {
            String::new()
        } else {
            format!(
                " AND child.kind IN ({})",
                sql_placeholders(child_kinds.len())
            )
        };
        let connection = self.connection().borrow();
        let query = format!(
            "SELECT child.*,
                    link.parent_action_id AS parent_action_id,
                    link.child_action_id AS child_action_id,
                    link.role AS role,
                    link.confidence AS confidence,
                    link.attributes AS link_attributes
             FROM semantic_action_links link
             JOIN semantic_actions child
               ON child.trace_id = link.trace_id
              AND child.action_id = link.child_action_id
             WHERE link.trace_id = ?
               AND link.parent_action_id = ?
               AND link.role IN ({})
               {}
             ORDER BY child.start_time ASC, child.action_id ASC, link.role ASC",
            sql_placeholders(roles.len()),
            kind_filter
        );
        let values = role_and_kind_query_values(trace_id, parent_action_id, roles, child_kinds)?;
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
                || invalidated_link_attrs(
                    &child.link.attributes,
                    child.link.role.as_str(),
                    &child.action.attributes,
                )
                || !seen.insert(child.action.action_id.clone())
            {
                continue;
            }
            children.push(child);
        }
        load_child_metadata(&connection, trace_id, &mut children, child_roles)?;
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
        role: decode_link_role(row.get::<_, String>("role")?)?,
        confidence: decode_link_confidence(row.get::<_, String>("confidence")?)?,
        evidence: Vec::new(),
        attributes: decode_map(&row.get::<_, String>("link_attributes")?),
    };
    Ok(SemanticActionChildRow {
        action,
        link,
        child_count: 0,
    })
}

fn role_and_kind_query_values(
    trace_id: TraceId,
    parent_action_id: &str,
    roles: &[&str],
    child_kinds: &[&str],
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let trace_id = i64::try_from(trace_id.get()).map_err(|error| {
        SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
    })?;
    let mut values = Vec::with_capacity(2 + roles.len() + child_kinds.len());
    values.push(Value::Integer(trace_id));
    values.push(Value::Text(parent_action_id.to_string()));
    values.extend(roles.iter().map(|role| Value::Text((*role).to_string())));
    values.extend(
        child_kinds
            .iter()
            .map(|kind| Value::Text((*kind).to_string())),
    );
    Ok(values)
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

//! Root-level semantic action tree queries.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params_from_iter};
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
    SemanticActionStoreError,
};

use crate::SqliteStorage;
use crate::records::decode_map;
use crate::semantic_actions::codebook::sqlite::{
    action_kind_code, decode_link_confidence, decode_link_role, link_role_code,
    link_role_code_from_str,
};
use crate::semantic_actions::cold_fields::decode_text_from_row;
use crate::semantic_actions::store::{
    ACTION_SELECT_COLUMNS, LINK_SELECT_COLUMNS, action_cold_field_join, action_from_row,
    link_cold_field_join, read_link_evidence,
};
use crate::semantic_actions::tree::SemanticActionChildPageQuery;
use crate::semantic_actions::tree_metadata::{
    display_child_counts, display_parent_link_absence_predicate, display_parent_link_value_count,
    push_display_parent_link_values,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionDisplayRootChildRow {
    pub root_link: Option<SemanticActionLink>,
    pub action: SemanticAction,
    pub child_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionDisplayRootChildPage {
    pub rows: Vec<SemanticActionDisplayRootChildRow>,
    pub total_count: usize,
}

impl SqliteStorage {
    pub fn semantic_action_display_root_child_count(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
    ) -> Result<usize, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        root_child_count(&connection, trace_id, display_parent_roles)
    }

    pub fn semantic_action_display_root_children_page(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
        root_link_roles: &[&str],
        page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionDisplayRootChildPage, SemanticActionStoreError> {
        ensure_semantic_trace(self, trace_id)?;
        let connection = self.connection().borrow();
        let total_count = root_child_count(&connection, trace_id, display_parent_roles)?;
        let action_ids = root_child_action_ids(
            &connection,
            trace_id,
            display_parent_roles,
            page.offset,
            page.limit,
        )?;
        let mut actions = read_actions(&connection, trace_id, &action_ids)?;
        let mut root_links = read_root_links(&connection, trace_id, &action_ids, root_link_roles)?;
        let mut child_counts =
            root_child_counts(&connection, trace_id, &actions, display_parent_roles)?;
        let rows = action_ids
            .into_iter()
            .filter_map(|action_id| {
                actions
                    .remove(&action_id)
                    .map(|action| SemanticActionDisplayRootChildRow {
                        root_link: root_links.remove(&action_id),
                        child_count: child_counts.remove(&action_id).unwrap_or_default(),
                        action,
                    })
            })
            .collect();
        Ok(SemanticActionDisplayRootChildPage { rows, total_count })
    }
}

fn root_child_count(
    connection: &Connection,
    trace_id: TraceId,
    display_parent_roles: &[&str],
) -> Result<usize, SemanticActionStoreError> {
    let query = format!(
        "SELECT COUNT(*)
         FROM semantic_actions action
         WHERE {}",
        root_candidate_predicate(display_parent_roles)
    );
    let values = root_candidate_values(trace_id, display_parent_roles)?;
    let count = connection
        .query_row(&query, params_from_iter(values), |row| row.get::<_, i64>(0))
        .map_err(|error| {
            SemanticActionStoreError::new("count_semantic_action_root_children", error.to_string())
        })?;
    usize::try_from(count).map_err(|error| {
        SemanticActionStoreError::new("count_semantic_action_root_children", error.to_string())
    })
}

fn root_child_action_ids(
    connection: &Connection,
    trace_id: TraceId,
    display_parent_roles: &[&str],
    offset: usize,
    limit: usize,
) -> Result<Vec<String>, SemanticActionStoreError> {
    let query = format!(
        "SELECT ids.action_id
         FROM semantic_actions action
         JOIN semantic_action_ids ids
           ON ids.action_key = action.action_key
         WHERE {}
         ORDER BY action.start_time ASC, ids.action_id ASC
         LIMIT ? OFFSET ?",
        root_candidate_predicate(display_parent_roles)
    );
    let mut values = root_candidate_values(trace_id, display_parent_roles)?;
    values.push(usize_value(limit, "semantic_action_root_child_limit")?);
    values.push(usize_value(offset, "semantic_action_root_child_offset")?);
    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new("prepare_semantic_action_root_children", error.to_string())
    })?;
    let rows = statement
        .query_map(params_from_iter(values), |row| row.get::<_, String>(0))
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_root_children", error.to_string())
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        SemanticActionStoreError::new("map_semantic_action_root_child", error.to_string())
    })
}

fn read_actions(
    connection: &Connection,
    trace_id: TraceId,
    action_ids: &[String],
) -> Result<BTreeMap<String, SemanticAction>, SemanticActionStoreError> {
    if action_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let query = format!(
        "SELECT {ACTION_SELECT_COLUMNS}
         FROM semantic_actions action
         JOIN semantic_action_ids ids
           ON ids.action_key = action.action_key
         {}
         WHERE action.trace_id = ?
           AND ids.action_id IN ({})",
        action_cold_field_join(),
        sql_placeholders(action_ids.len())
    );
    let mut values = Vec::with_capacity(1 + action_ids.len());
    values.push(trace_id_value(trace_id)?);
    values.extend(action_ids.iter().cloned().map(Value::Text));
    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new("prepare_semantic_action_root_action", error.to_string())
    })?;
    let rows = statement
        .query_map(params_from_iter(values), action_from_row)
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_root_action", error.to_string())
        })?;
    let mut actions = BTreeMap::new();
    for row in rows {
        let action = row.map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_root_action", error.to_string())
        })?;
        actions.insert(action.action_id.clone(), action);
    }
    Ok(actions)
}

fn read_root_links(
    connection: &Connection,
    trace_id: TraceId,
    action_ids: &[String],
    root_link_roles: &[&str],
) -> Result<BTreeMap<String, SemanticActionLink>, SemanticActionStoreError> {
    if action_ids.is_empty() || root_link_roles.is_empty() {
        return Ok(BTreeMap::new());
    }
    let query = format!(
        "SELECT {LINK_SELECT_COLUMNS}
         FROM semantic_action_links link
         JOIN semantic_actions parent
           ON parent.action_key = link.parent_action_key
         JOIN semantic_action_ids parent_ids
           ON parent_ids.action_key = link.parent_action_key
         JOIN semantic_actions child
           ON child.action_key = link.child_action_key
         JOIN semantic_action_ids child_ids
           ON child_ids.action_key = link.child_action_key
         {}
         WHERE link.trace_id = ?
           AND child_ids.action_id IN ({})
           AND link.role_code IN ({})
           AND link.link_valid_code = 1
           AND parent.action_valid_code = 1
           AND child.action_valid_code = 1
           AND NOT (
             (link.role_code = ? OR link.role_code = ?)
             AND child.process_parent_conflict = 1
           )
         ORDER BY child.start_time ASC, child_ids.action_id ASC, link.role_code ASC, parent_ids.action_id ASC",
        link_cold_field_join(),
        sql_placeholders(action_ids.len()),
        sql_placeholders(root_link_roles.len())
    );
    let mut values = Vec::with_capacity(1 + action_ids.len() + root_link_roles.len() + 2);
    values.push(trace_id_value(trace_id)?);
    values.extend(action_ids.iter().cloned().map(Value::Text));
    for role in root_link_roles {
        values.push(Value::Integer(i64::from(link_role_code_from_str(role)?)));
    }
    values.push(Value::Integer(i64::from(link_role_code(
        SemanticActionLinkRole::AgentPerformedAction,
    ))));
    values.push(Value::Integer(i64::from(link_role_code(
        SemanticActionLinkRole::CommandContainsCommandInvocation,
    ))));

    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new("prepare_semantic_action_root_links", error.to_string())
    })?;
    let rows = statement
        .query_map(params_from_iter(values), root_link_from_row)
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_root_links", error.to_string())
        })?;
    let mut links = BTreeMap::new();
    for row in rows {
        let mut link = row.map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_root_link", error.to_string())
        })?;
        link.evidence = read_link_evidence(connection, &link)?;
        links.entry(link.child_action_id.clone()).or_insert(link);
    }
    Ok(links)
}

fn root_child_counts(
    connection: &Connection,
    trace_id: TraceId,
    actions: &BTreeMap<String, SemanticAction>,
    display_parent_roles: &[&str],
) -> Result<BTreeMap<String, usize>, SemanticActionStoreError> {
    let action_ids = actions.keys().cloned().collect::<Vec<_>>();
    let mut counts = display_child_counts(connection, trace_id, &action_ids, display_parent_roles)?;
    let command_ids = actions
        .values()
        .filter(|action| action.kind == SemanticActionKind::CommandInvocation)
        .map(|action| action.action_id.clone())
        .collect::<Vec<_>>();
    for (action_id, fallback_count) in
        command_fallback_child_counts(connection, trace_id, &command_ids, display_parent_roles)?
    {
        *counts.entry(action_id).or_default() += fallback_count;
    }
    Ok(counts)
}

fn command_fallback_child_counts(
    connection: &Connection,
    trace_id: TraceId,
    command_ids: &[String],
    display_parent_roles: &[&str],
) -> Result<BTreeMap<String, usize>, SemanticActionStoreError> {
    if command_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let parent_filter = display_parent_link_absence_predicate(display_parent_roles, "child");
    let query = format!(
        "SELECT command_ids.action_id, COUNT(DISTINCT child.action_key)
         FROM semantic_actions command
         JOIN semantic_action_ids command_ids
           ON command_ids.action_key = command.action_key
         JOIN semantic_actions child
           ON child.trace_id = command.trace_id
          AND child.kind_code != ?
          AND child.process_pid = command.process_pid
          AND child.process_task_id IS command.process_task_id
          AND child.process_start_ticks = command.process_start_ticks
          AND child.process_pid_namespace IS command.process_pid_namespace
          AND child.process_generation = command.process_generation
          AND child.start_time >= command.start_time
          AND (command.end_time IS NULL OR child.start_time <= command.end_time)
         WHERE command.trace_id = ?
           AND command_ids.action_id IN ({})
           AND command.kind_code = ?
           AND command.action_valid_code = 1
           AND child.action_valid_code = 1
           {}
         GROUP BY command_ids.action_id",
        sql_placeholders(command_ids.len()),
        parent_filter
    );
    let mut values = Vec::with_capacity(
        3 + command_ids.len() + display_parent_link_value_count(display_parent_roles),
    );
    values.push(Value::Integer(i64::from(action_kind_code(
        SemanticActionKind::CommandInvocation,
    ))));
    values.push(trace_id_value(trace_id)?);
    values.extend(command_ids.iter().cloned().map(Value::Text));
    values.push(Value::Integer(i64::from(action_kind_code(
        SemanticActionKind::CommandInvocation,
    ))));
    push_display_parent_link_values(&mut values, display_parent_roles)?;

    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new(
            "prepare_semantic_action_root_fallback_counts",
            error.to_string(),
        )
    })?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_root_fallback_counts",
                error.to_string(),
            )
        })?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let (action_id, count) = row.map_err(|error| {
            SemanticActionStoreError::new(
                "map_semantic_action_root_fallback_counts",
                error.to_string(),
            )
        })?;
        counts.insert(
            action_id,
            usize::try_from(count).map_err(|error| {
                SemanticActionStoreError::new(
                    "map_semantic_action_root_fallback_counts",
                    error.to_string(),
                )
            })?,
        );
    }
    Ok(counts)
}

fn root_candidate_predicate(display_parent_roles: &[&str]) -> String {
    let parent_filter = display_parent_link_absence_predicate(display_parent_roles, "action");
    format!(
        "action.trace_id = ?
         AND action.action_valid_code = 1
         {}
         AND (
           action.kind_code = ?
           OR NOT EXISTS (
             SELECT 1
             FROM semantic_actions command
             WHERE command.trace_id = action.trace_id
               AND command.kind_code = ?
               AND command.action_key != action.action_key
               AND command.process_pid = action.process_pid
               AND command.process_task_id IS action.process_task_id
               AND command.process_start_ticks = action.process_start_ticks
               AND command.process_pid_namespace IS action.process_pid_namespace
               AND command.process_generation = action.process_generation
               AND command.start_time <= action.start_time
               AND (command.end_time IS NULL OR action.start_time <= command.end_time)
               AND command.action_valid_code = 1
           )
         )",
        parent_filter
    )
}

fn root_candidate_values(
    trace_id: TraceId,
    display_parent_roles: &[&str],
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let mut values = Vec::with_capacity(3 + display_parent_link_value_count(display_parent_roles));
    values.push(trace_id_value(trace_id)?);
    push_display_parent_link_values(&mut values, display_parent_roles)?;
    values.push(Value::Integer(i64::from(action_kind_code(
        SemanticActionKind::CommandInvocation,
    ))));
    values.push(Value::Integer(i64::from(action_kind_code(
        SemanticActionKind::CommandInvocation,
    ))));
    Ok(values)
}

fn root_link_from_row(row: &Row<'_>) -> Result<SemanticActionLink, rusqlite::Error> {
    Ok(SemanticActionLink {
        trace_id: TraceId::new(row.get("trace_id")?),
        parent_action_id: row.get("parent_action_id")?,
        child_action_id: row.get("child_action_id")?,
        role: decode_link_role(row.get::<_, i64>("role_code")?)?,
        confidence: decode_link_confidence(row.get::<_, i64>("confidence_code")?)?,
        valid: row.get("valid")?,
        evidence: Vec::new(),
        attributes: decode_map(&decode_text_from_row(row, "legacy_attributes")?),
    })
}

fn ensure_semantic_trace(
    storage: &SqliteStorage,
    trace_id: TraceId,
) -> Result<(), SemanticActionStoreError> {
    if storage.is_purged(trace_id) {
        return Err(SemanticActionStoreError::new(
            "read_semantic_action_root_children",
            "trace has been purged",
        ));
    }
    Ok(())
}

fn trace_id_value(trace_id: TraceId) -> Result<Value, SemanticActionStoreError> {
    i64::try_from(trace_id.get())
        .map(Value::Integer)
        .map_err(|error| {
            SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
        })
}

fn usize_value(value: usize, stage: &'static str) -> Result<Value, SemanticActionStoreError> {
    i64::try_from(value)
        .map(Value::Integer)
        .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

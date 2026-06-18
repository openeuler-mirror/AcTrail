//! Root-level semantic action tree queries.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params_from_iter};
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionStoreError,
    attr_keys as attrs, link_roles,
};

use crate::SqliteStorage;
use crate::records::decode_map;
use crate::semantic_actions::store::{
    action_from_row, decode_link_confidence, decode_link_role, read_link_evidence,
};
use crate::semantic_actions::tree::SemanticActionChildPageQuery;
use crate::semantic_actions::tree_metadata::display_child_counts;

const ACTION_INVALID_MARKER: &str = attrs::actrail::ACTION_VALID_FALSE_MARKER;
const LINK_INVALID_MARKER: &str = attrs::actrail::LINK_VALID_FALSE_MARKER;
const PARENT_CONFLICT_MARKER: &str = attrs::process_parent::IDENTITY_STATE_CONFLICT_MARKER;
const AGENT_ROOT_ROLE: &str = link_roles::AGENT_PERFORMED_ACTION;
const COMMAND_CONTAINS_COMMAND_ROLE: &str = link_roles::COMMAND_CONTAINS_COMMAND_INVOCATION;
const COMMAND_KIND: &str = SemanticActionKind::CommandInvocation.as_str();

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
        "SELECT action.action_id
         FROM semantic_actions action
         WHERE {}
         ORDER BY action.start_time ASC, action.action_id ASC
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
        "SELECT *
         FROM semantic_actions
         WHERE trace_id = ?
           AND action_id IN ({})",
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
        "SELECT link.*
         FROM semantic_action_links link
         JOIN semantic_actions parent
           ON parent.trace_id = link.trace_id
          AND parent.action_id = link.parent_action_id
         JOIN semantic_actions child
           ON child.trace_id = link.trace_id
          AND child.action_id = link.child_action_id
         WHERE link.trace_id = ?
           AND link.child_action_id IN ({})
           AND link.role IN ({})
           AND link.valid = 1
           AND instr(link.attributes, ?) = 0
           AND instr(parent.attributes, ?) = 0
           AND instr(child.attributes, ?) = 0
           AND NOT (
             (link.role = ? OR link.role = ?)
             AND instr(child.attributes, ?) > 0
           )
         ORDER BY child.start_time ASC, child.action_id ASC, link.role ASC, link.parent_action_id ASC",
        sql_placeholders(action_ids.len()),
        sql_placeholders(root_link_roles.len())
    );
    let mut values = Vec::with_capacity(1 + action_ids.len() + root_link_roles.len() + 6);
    values.push(trace_id_value(trace_id)?);
    values.extend(action_ids.iter().cloned().map(Value::Text));
    values.extend(
        root_link_roles
            .iter()
            .map(|role| Value::Text((*role).to_string())),
    );
    values.push(Value::Text(LINK_INVALID_MARKER.to_string()));
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    values.push(Value::Text(AGENT_ROOT_ROLE.to_string()));
    values.push(Value::Text(COMMAND_CONTAINS_COMMAND_ROLE.to_string()));
    values.push(Value::Text(PARENT_CONFLICT_MARKER.to_string()));

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
    let parent_filter = valid_display_parent_link_exists_predicate(display_parent_roles, "child");
    let query = format!(
        "SELECT command.action_id, COUNT(DISTINCT child.action_id)
         FROM semantic_actions command
         JOIN semantic_actions child
           ON child.trace_id = command.trace_id
          AND child.kind != ?
          AND child.process_pid = command.process_pid
          AND child.process_task_id IS command.process_task_id
          AND child.process_start_ticks = command.process_start_ticks
          AND child.process_pid_namespace IS command.process_pid_namespace
          AND child.process_generation = command.process_generation
          AND child.start_time >= command.start_time
          AND (command.end_time IS NULL OR child.start_time <= command.end_time)
         WHERE command.trace_id = ?
           AND command.action_id IN ({})
           AND command.kind = ?
           AND instr(command.attributes, ?) = 0
           AND instr(child.attributes, ?) = 0
           {}
         GROUP BY command.action_id",
        sql_placeholders(command_ids.len()),
        parent_filter
    );
    let mut values = Vec::with_capacity(
        4 + command_ids.len() + link_predicate_value_count(display_parent_roles),
    );
    values.push(Value::Text(COMMAND_KIND.to_string()));
    values.push(trace_id_value(trace_id)?);
    values.extend(command_ids.iter().cloned().map(Value::Text));
    values.push(Value::Text(COMMAND_KIND.to_string()));
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    push_link_predicate_values(&mut values, display_parent_roles);

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
    let parent_filter = valid_display_parent_link_exists_predicate(display_parent_roles, "action");
    format!(
        "action.trace_id = ?
         AND instr(action.attributes, ?) = 0
         {}
         AND (
           action.kind = ?
           OR NOT EXISTS (
             SELECT 1
             FROM semantic_actions command
             WHERE command.trace_id = action.trace_id
               AND command.kind = ?
               AND command.action_id != action.action_id
               AND command.process_pid = action.process_pid
               AND command.process_task_id IS action.process_task_id
               AND command.process_start_ticks = action.process_start_ticks
               AND command.process_pid_namespace IS action.process_pid_namespace
               AND command.process_generation = action.process_generation
               AND command.start_time <= action.start_time
               AND (command.end_time IS NULL OR action.start_time <= command.end_time)
               AND instr(command.attributes, ?) = 0
           )
         )",
        parent_filter
    )
}

fn root_candidate_values(
    trace_id: TraceId,
    display_parent_roles: &[&str],
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let mut values = Vec::with_capacity(5 + link_predicate_value_count(display_parent_roles));
    values.push(trace_id_value(trace_id)?);
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    push_link_predicate_values(&mut values, display_parent_roles);
    values.push(Value::Text(COMMAND_KIND.to_string()));
    values.push(Value::Text(COMMAND_KIND.to_string()));
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    Ok(values)
}

fn valid_display_parent_link_exists_predicate(roles: &[&str], child_alias: &str) -> String {
    if roles.is_empty() {
        return String::new();
    }
    format!(
        "AND NOT EXISTS (
           SELECT 1
           FROM semantic_action_links link
           JOIN semantic_actions parent
             ON parent.trace_id = link.trace_id
            AND parent.action_id = link.parent_action_id
           WHERE link.trace_id = {child_alias}.trace_id
             AND link.child_action_id = {child_alias}.action_id
             AND link.role IN ({})
             AND link.valid = 1
             AND instr(link.attributes, ?) = 0
             AND instr(parent.attributes, ?) = 0
             AND NOT (
               (link.role = ? OR link.role = ?)
               AND instr({child_alias}.attributes, ?) > 0
             )
         )",
        sql_placeholders(roles.len())
    )
}

fn push_link_predicate_values(values: &mut Vec<Value>, roles: &[&str]) {
    if roles.is_empty() {
        return;
    }
    values.extend(roles.iter().map(|role| Value::Text((*role).to_string())));
    values.push(Value::Text(LINK_INVALID_MARKER.to_string()));
    values.push(Value::Text(ACTION_INVALID_MARKER.to_string()));
    values.push(Value::Text(AGENT_ROOT_ROLE.to_string()));
    values.push(Value::Text(COMMAND_CONTAINS_COMMAND_ROLE.to_string()));
    values.push(Value::Text(PARENT_CONFLICT_MARKER.to_string()));
}

fn link_predicate_value_count(roles: &[&str]) -> usize {
    if roles.is_empty() { 0 } else { roles.len() + 5 }
}

fn root_link_from_row(row: &Row<'_>) -> Result<SemanticActionLink, rusqlite::Error> {
    Ok(SemanticActionLink {
        trace_id: TraceId::new(row.get("trace_id")?),
        parent_action_id: row.get("parent_action_id")?,
        child_action_id: row.get("child_action_id")?,
        role: decode_link_role(row.get::<_, String>("role")?)?,
        confidence: decode_link_confidence(row.get::<_, String>("confidence")?)?,
        valid: row.get("valid")?,
        evidence: Vec::new(),
        attributes: decode_map(&row.get::<_, String>("attributes")?),
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

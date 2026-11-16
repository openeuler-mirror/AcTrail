//! Indexed navigation through the semantic-action display tree.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use semantic_action::{SemanticActionKind, SemanticActionStoreError};
use storage_core::SemanticActionDisplayPathEntry;

use crate::SqliteStorage;
use crate::semantic_actions::codebook::sqlite::{
    action_kind_code_from_str, decode_kind, link_role_code_from_str,
};

#[derive(Clone, Debug)]
struct ActionPosition {
    action_key: i64,
    action_id: String,
    start_time: i64,
    kind: SemanticActionKind,
}

impl SqliteStorage {
    pub fn semantic_action_display_path_to_kind(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
        selected_parent_roles: &[&str],
        target_kind: &str,
        after_action_id: Option<&str>,
    ) -> Result<Option<Vec<SemanticActionDisplayPathEntry>>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "semantic_action_display_path_to_kind",
                "trace has been purged",
            ));
        }
        if display_parent_roles.is_empty() {
            return Ok(None);
        }
        let connection = self.connection().borrow();
        let target = navigation_target(
            &connection,
            trace_id,
            target_kind,
            after_action_id.filter(|value| !value.is_empty()),
        )?;
        target
            .map(|target| {
                display_path(
                    &connection,
                    trace_id,
                    target,
                    display_parent_roles,
                    selected_parent_roles,
                )
            })
            .transpose()
    }

    /// The display path to one action, like `semantic_action_display_path_to_kind`.
    pub fn semantic_action_display_path_to_action(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
        selected_parent_roles: &[&str],
        action_id: &str,
    ) -> Result<Option<Vec<SemanticActionDisplayPathEntry>>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "semantic_action_display_path_to_action",
                "trace has been purged",
            ));
        }
        if display_parent_roles.is_empty() {
            return Ok(None);
        }
        let connection = self.connection().borrow();
        action_position_by_id(&connection, trace_id, action_id)?
            .map(|target| {
                display_path(
                    &connection,
                    trace_id,
                    target,
                    display_parent_roles,
                    selected_parent_roles,
                )
            })
            .transpose()
    }
}

fn display_path(
    connection: &Connection,
    trace_id: TraceId,
    mut current: ActionPosition,
    display_parent_roles: &[&str],
    selected_parent_roles: &[&str],
) -> Result<Vec<SemanticActionDisplayPathEntry>, SemanticActionStoreError> {
    let mut reverse_path = Vec::new();
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current.action_key) {
            return Err(SemanticActionStoreError::new(
                "semantic_action_display_path_to_kind",
                "cycle in semantic action display parents",
            ));
        }
        let parent = selected_parent(
            connection,
            trace_id,
            current.action_key,
            selected_parent_roles,
        )?;
        let offset = sibling_offset(
            connection,
            trace_id,
            parent.as_ref().map(|value| value.action_key),
            &current,
            display_parent_roles,
        )?;
        reverse_path.push(SemanticActionDisplayPathEntry {
            parent_action_id: parent.as_ref().map(|value| value.action_id.clone()),
            action_id: current.action_id,
            offset,
            kind: current.kind,
        });
        let Some(parent) = parent else {
            break;
        };
        current = parent;
    }
    reverse_path.reverse();
    Ok(reverse_path)
}

fn navigation_target(
    connection: &Connection,
    trace_id: TraceId,
    target_kind: &str,
    after_action_id: Option<&str>,
) -> Result<Option<ActionPosition>, SemanticActionStoreError> {
    let kind_code = i64::from(action_kind_code_from_str(target_kind)?);
    let anchor = match after_action_id {
        Some(action_id) => action_position_by_id(connection, trace_id, action_id)?,
        None => None,
    };
    let (after_start, after_id) = anchor
        .as_ref()
        .map(|value| (Some(value.start_time), Some(value.action_id.as_str())))
        .unwrap_or((None, None));
    let mut statement = connection
        .prepare(
            "SELECT action.action_key, ids.action_id, action.start_time, action.kind_code
             FROM semantic_actions action
             JOIN semantic_action_ids ids ON ids.action_key = action.action_key
             WHERE action.trace_id = ?1
               AND action.kind_code = ?2
               AND action.action_valid_code = 1
               AND (?3 IS NULL OR action.start_time > ?3
                    OR (action.start_time = ?3 AND ids.action_id > ?4))
             ORDER BY action.start_time ASC, ids.action_id ASC
             LIMIT 1",
        )
        .map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_navigation_target",
                error.to_string(),
            )
        })?;
    statement
        .query_row(
            params![trace_id.get(), kind_code, after_start, after_id],
            action_position_from_row,
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_navigation_target",
                error.to_string(),
            )
        })
}

fn action_position_by_id(
    connection: &Connection,
    trace_id: TraceId,
    action_id: &str,
) -> Result<Option<ActionPosition>, SemanticActionStoreError> {
    connection
        .query_row(
            "SELECT action.action_key, ids.action_id, action.start_time, action.kind_code
             FROM semantic_actions action
             JOIN semantic_action_ids ids ON ids.action_key = action.action_key
             WHERE action.trace_id = ?1 AND ids.action_id = ?2
             LIMIT 1",
            params![trace_id.get(), action_id],
            action_position_from_row,
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_navigation_anchor",
                error.to_string(),
            )
        })
}

fn selected_parent(
    connection: &Connection,
    trace_id: TraceId,
    child_action_key: i64,
    roles: &[&str],
) -> Result<Option<ActionPosition>, SemanticActionStoreError> {
    if roles.is_empty() {
        return Ok(None);
    }
    let role_codes = role_codes(roles)?;
    let query = format!(
        "SELECT parent.action_key, parent_ids.action_id, parent.start_time, parent.kind_code
         FROM semantic_action_links link
         JOIN semantic_actions parent ON parent.action_key = link.parent_action_key
         JOIN semantic_action_ids parent_ids ON parent_ids.action_key = parent.action_key
         WHERE link.trace_id = ?1
           AND link.child_action_key = ?2
           AND link.valid = 1
           AND parent.action_valid_code = 1
           AND link.role_code IN ({})
         ORDER BY {} ASC, parent.start_time ASC, parent_ids.action_id ASC
         LIMIT 1",
        sql_integers(&role_codes),
        role_rank_sql("link.role_code", &role_codes),
    );
    let values = vec![
        Value::Integer(i64::try_from(trace_id.get()).map_err(|error| {
            SemanticActionStoreError::new("semantic_action_navigation_trace_id", error.to_string())
        })?),
        Value::Integer(child_action_key),
    ];
    connection
        .query_row(&query, params_from_iter(values), action_position_from_row)
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_navigation_parent",
                error.to_string(),
            )
        })
}

fn sibling_offset(
    connection: &Connection,
    trace_id: TraceId,
    parent_action_key: Option<i64>,
    child: &ActionPosition,
    roles: &[&str],
) -> Result<usize, SemanticActionStoreError> {
    let role_codes = role_codes(roles)?;
    let count: i64 = if let Some(parent_action_key) = parent_action_key {
        let query = format!(
            "SELECT COUNT(DISTINCT candidate.action_key)
             FROM semantic_action_links link
             JOIN semantic_actions candidate ON candidate.action_key = link.child_action_key
             JOIN semantic_action_ids candidate_ids ON candidate_ids.action_key = candidate.action_key
             WHERE link.trace_id = ?1
               AND link.parent_action_key = ?2
               AND link.valid = 1
               AND candidate.action_valid_code = 1
               AND link.role_code IN ({})
               AND (candidate.start_time < ?3
                    OR (candidate.start_time = ?3 AND candidate_ids.action_id < ?4))",
            sql_integers(&role_codes),
        );
        connection.query_row(
            &query,
            params![
                trace_id.get(),
                parent_action_key,
                child.start_time,
                child.action_id
            ],
            |row| row.get(0),
        )
    } else {
        return super::root_tree::DisplayRootQuery::new(trace_id, roles)
            .offset_before(connection, child.start_time, &child.action_id);
    }
    .map_err(|error| {
        SemanticActionStoreError::new(
            "query_semantic_action_navigation_offset",
            error.to_string(),
        )
    })?;
    usize::try_from(count).map_err(|error| {
        SemanticActionStoreError::new("map_semantic_action_navigation_offset", error.to_string())
    })
}

fn action_position_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActionPosition> {
    Ok(ActionPosition {
        action_key: row.get(0)?,
        action_id: row.get(1)?,
        start_time: row.get(2)?,
        kind: decode_kind(row.get(3)?)?,
    })
}

fn role_codes(roles: &[&str]) -> Result<Vec<i16>, SemanticActionStoreError> {
    roles
        .iter()
        .map(|role| link_role_code_from_str(role))
        .collect()
}

fn role_rank_sql(column: &str, role_codes: &[i16]) -> String {
    let cases = role_codes
        .iter()
        .enumerate()
        .map(|(rank, code)| format!("WHEN {code} THEN {rank}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("CASE {column} {cases} ELSE {} END", role_codes.len())
}

fn sql_integers(values: &[i16]) -> String {
    values
        .iter()
        .map(i16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
#[path = "../../../../../../tests/unit/crates/storage/adapters/sqlite/semantic_actions/navigation/tests.rs"]
mod tests;

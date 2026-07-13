//! Command fallback child queries for action tree display.

use model_core::ids::TraceId;
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStoreError};

use crate::SqliteStorage;
use crate::records::encode_time;
use crate::semantic_actions::codebook::sqlite::action_kind_code;
use crate::semantic_actions::store::{
    ACTION_SELECT_COLUMNS, action_cold_field_join, action_from_row, read_evidence,
};
use crate::semantic_actions::tree_metadata::{
    display_parent_link_absence_predicate, display_parent_link_value_count,
    invalidated_action_attrs, push_display_parent_link_values,
};

impl SqliteStorage {
    pub fn semantic_action_command_fallback_children(
        &self,
        trace_id: TraceId,
        command: &SemanticAction,
        display_parent_roles: &[&str],
    ) -> Result<Vec<SemanticAction>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "read_semantic_actions",
                "trace has been purged",
            ));
        }
        if display_parent_roles.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection().borrow();
        let action_cold_join = action_cold_field_join();
        let parent_filter = display_parent_link_absence_predicate(display_parent_roles, "action");
        let query = format!(
            "SELECT {ACTION_SELECT_COLUMNS}
             FROM semantic_actions action
             JOIN semantic_action_ids ids
               ON ids.action_key = action.action_key
             {action_cold_join}
             WHERE action.trace_id = ?
               AND action.kind_code != ?
               AND action.process_id = ?
               AND action.start_time >= ?
               AND (? IS NULL OR action.start_time <= ?)
               AND action.action_valid_code = 1
               {parent_filter}
             ORDER BY action.start_time ASC, ids.action_id ASC",
        );
        let values = command_fallback_query_values(trace_id, command, display_parent_roles)?;
        let mut statement = connection.prepare(&query).map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_command_fallback_children",
                error.to_string(),
            )
        })?;
        let rows = statement
            .query_map(params_from_iter(values), action_from_row)
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "query_semantic_action_command_fallback_children",
                    error.to_string(),
                )
            })?;
        let mut children = Vec::new();
        for row in rows {
            let mut action = row.map_err(|error| {
                SemanticActionStoreError::new(
                    "map_semantic_action_command_fallback_child",
                    error.to_string(),
                )
            })?;
            if invalidated_action_attrs(&action.attributes) {
                continue;
            }
            action.evidence = read_evidence(&connection, &action.action_id)?;
            children.push(action);
        }
        Ok(children)
    }
}

fn command_fallback_query_values(
    trace_id: TraceId,
    command: &SemanticAction,
    display_parent_roles: &[&str],
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let end_time = command.end_time.map(encode_time);
    let mut values = Vec::with_capacity(6 + display_parent_link_value_count(display_parent_roles));
    values.extend([
        trace_id_value(trace_id)?,
        Value::Integer(i64::from(action_kind_code(
            SemanticActionKind::CommandInvocation,
        ))),
        Value::Integer(i64::try_from(command.process.get()).map_err(|error| {
            SemanticActionStoreError::new(
                "semantic_action_command_fallback_process_id",
                error.to_string(),
            )
        })?),
        Value::Integer(encode_time(command.start_time)),
        end_time.map_or(Value::Null, Value::Integer),
        end_time.map_or(Value::Null, Value::Integer),
    ]);
    push_display_parent_link_values(&mut values, display_parent_roles)?;
    Ok(values)
}

fn trace_id_value(trace_id: TraceId) -> Result<Value, SemanticActionStoreError> {
    i64::try_from(trace_id.get())
        .map(Value::Integer)
        .map_err(|error| {
            SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
        })
}

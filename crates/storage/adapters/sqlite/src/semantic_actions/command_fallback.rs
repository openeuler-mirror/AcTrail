//! Command fallback child queries for action tree display.

use model_core::ids::TraceId;
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use semantic_action::{SemanticAction, SemanticActionStoreError};

use crate::SqliteStorage;
use crate::records::encode_time;
use crate::semantic_actions::store::{action_from_row, read_evidence};
use crate::semantic_actions::tree_metadata::invalidated_action_attrs;

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
        let query = format!(
            "SELECT child.*
             FROM semantic_actions child
             WHERE child.trace_id = ?
               AND child.kind != 'command.invocation'
               AND child.process_pid = ?
               AND child.process_task_id IS ?
               AND child.process_start_ticks = ?
               AND child.process_pid_namespace IS ?
               AND child.process_generation = ?
               AND child.start_time >= ?
               AND (? IS NULL OR child.start_time <= ?)
               AND NOT EXISTS (
                   SELECT 1 FROM semantic_action_links link
                   WHERE link.trace_id = child.trace_id
                     AND link.child_action_id = child.action_id
                     AND link.valid = 1
                     AND link.role IN ({})
               )
             ORDER BY child.start_time ASC, child.action_id ASC",
            sql_placeholders(display_parent_roles.len())
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
    let mut values = vec![
        trace_id_value(trace_id)?,
        Value::Integer(i64::from(command.process.pid)),
        command
            .process
            .task_id
            .map(|task_id| Value::Integer(i64::from(task_id)))
            .unwrap_or(Value::Null),
        Value::Integer(
            i64::try_from(command.process.start_time_ticks).map_err(|error| {
                SemanticActionStoreError::new(
                    "semantic_action_command_fallback_start_ticks",
                    error.to_string(),
                )
            })?,
        ),
        command
            .process
            .pid_namespace
            .as_ref()
            .map(|namespace| Value::Text(namespace.as_str().to_string()))
            .unwrap_or(Value::Null),
        Value::Integer(i64::try_from(command.process.generation).map_err(|error| {
            SemanticActionStoreError::new(
                "semantic_action_command_fallback_generation",
                error.to_string(),
            )
        })?),
        Value::Integer(encode_time(command.start_time)),
        end_time.map_or(Value::Null, Value::Integer),
        end_time.map_or(Value::Null, Value::Integer),
    ];
    values.extend(
        display_parent_roles
            .iter()
            .map(|role| Value::Text((*role).to_string())),
    );
    Ok(values)
}

fn trace_id_value(trace_id: TraceId) -> Result<Value, SemanticActionStoreError> {
    i64::try_from(trace_id.get())
        .map(Value::Integer)
        .map_err(|error| {
            SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
        })
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

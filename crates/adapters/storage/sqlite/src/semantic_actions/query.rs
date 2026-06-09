//! Targeted semantic action queries.

use model_core::ids::TraceId;
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use semantic_action::{SemanticAction, SemanticActionStoreError};

use crate::SqliteStorage;
use crate::semantic_actions::store::{action_from_row, read_evidence};

impl SqliteStorage {
    pub fn semantic_actions_matching_kinds(
        &self,
        trace_id: TraceId,
        kinds: &[&str],
    ) -> Result<Vec<SemanticAction>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "semantic_actions_matching_kinds",
                "trace has been purged",
            ));
        }
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let query = format!(
            "SELECT * FROM semantic_actions
             WHERE trace_id = ?
               AND kind IN ({})
             ORDER BY start_time ASC, action_id ASC",
            sql_placeholders(kinds.len())
        );
        let values = query_values(trace_id, kinds)?;
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(&query).map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_actions_matching_kinds",
                error.to_string(),
            )
        })?;
        let rows = statement
            .query_map(params_from_iter(values), action_from_row)
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "query_semantic_actions_matching_kinds",
                    error.to_string(),
                )
            })?;
        let mut actions = Vec::new();
        for row in rows {
            let mut action = row.map_err(|error| {
                SemanticActionStoreError::new(
                    "map_semantic_actions_matching_kinds",
                    error.to_string(),
                )
            })?;
            action.evidence = read_evidence(&connection, &action.action_id)?;
            actions.push(action);
        }
        Ok(actions)
    }
}

fn query_values(trace_id: TraceId, kinds: &[&str]) -> Result<Vec<Value>, SemanticActionStoreError> {
    let trace_id = i64::try_from(trace_id.get()).map_err(|error| {
        SemanticActionStoreError::new(
            "semantic_actions_matching_kinds_trace_id",
            error.to_string(),
        )
    })?;
    let mut values = Vec::with_capacity(1 + kinds.len());
    values.push(Value::Integer(trace_id));
    values.extend(kinds.iter().map(|kind| Value::Text((*kind).to_string())));
    Ok(values)
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

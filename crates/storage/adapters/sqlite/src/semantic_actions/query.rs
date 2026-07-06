//! Targeted semantic action queries.

use model_core::ids::TraceId;
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use semantic_action::{SemanticAction, SemanticActionStoreError};

use crate::SqliteStorage;
use crate::semantic_actions::codebook::sqlite::action_kind_code_from_str;
use crate::semantic_actions::store::{
    ACTION_SELECT_COLUMNS, action_cold_field_join, action_from_row, read_evidence,
};

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
            "SELECT {ACTION_SELECT_COLUMNS}
             FROM semantic_actions action
             JOIN semantic_action_ids ids
               ON ids.action_key = action.action_key
             {}
             WHERE action.trace_id = ?
               AND action.kind_code IN ({})
             ORDER BY action.start_time ASC, ids.action_id ASC",
            action_cold_field_join(),
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
    for kind in kinds {
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

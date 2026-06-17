//! Terminal-trace cleanup and tombstone persistence.

use rusqlite::params;
use store_retention_contract::RetentionError;
use store_retention_contract::cleanup::{RetentionCandidate, RetentionStore};
use store_retention_contract::tombstone::TraceTombstone;

use crate::SqliteStorage;
use crate::records::{
    decode_trace_health, decode_trace_lifecycle, encode_time, encode_trace_health,
    encode_trace_lifecycle,
};

impl RetentionStore for SqliteStorage {
    fn list_terminal_candidates(&self) -> Result<Vec<RetentionCandidate>, RetentionError> {
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare(
                "SELECT trace_id, lifecycle_state, health FROM traces
                 WHERE lifecycle_state IN ('completed', 'failed')
                 AND trace_id NOT IN (SELECT trace_id FROM tombstones)",
            )
            .map_err(|error| RetentionError::new("prepare_candidates", error.to_string()))?;
        let rows = statement
            .query_map([], |row| {
                Ok(RetentionCandidate {
                    trace_id: model_core::ids::TraceId::new(row.get(0)?),
                    lifecycle_state: decode_trace_lifecycle(&row.get::<_, String>(1)?)?,
                    health: decode_trace_health(&row.get::<_, String>(2)?)?,
                })
            })
            .map_err(|error| RetentionError::new("query_candidates", error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| RetentionError::new("map_candidates", error.to_string()))
    }

    fn purge_trace(
        &mut self,
        trace_id: model_core::ids::TraceId,
        tombstone: TraceTombstone,
    ) -> Result<(), RetentionError> {
        if self.export_leases().borrow().contains(&trace_id) {
            return Err(RetentionError::new(
                "purge_trace",
                "active export lease blocks purge",
            ));
        }
        let mut connection = self.connection().borrow_mut();
        let transaction = connection
            .transaction()
            .map_err(|error| RetentionError::new("begin_purge", error.to_string()))?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO tombstones (
                    trace_id, lifecycle_state, health, cleaned_at, cleanup_reason
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    tombstone.trace_id.get(),
                    encode_trace_lifecycle(tombstone.lifecycle_state),
                    encode_trace_health(tombstone.health),
                    encode_time(tombstone.cleaned_at),
                    tombstone.cleanup_reason,
                ],
            )
            .map_err(|error| RetentionError::new("insert_tombstone", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM events WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_events", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM semantic_action_link_evidence WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| {
                RetentionError::new("delete_semantic_action_link_evidence", error.to_string())
            })?;
        transaction
            .execute(
                "DELETE FROM semantic_action_links WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| {
                RetentionError::new("delete_semantic_action_links", error.to_string())
            })?;
        transaction
            .execute(
                "DELETE FROM semantic_action_evidence WHERE action_id IN (
                    SELECT action_id FROM semantic_actions WHERE trace_id = ?1
                )",
                params![trace_id.get()],
            )
            .map_err(|error| {
                RetentionError::new("delete_semantic_action_evidence", error.to_string())
            })?;
        transaction
            .execute(
                "DELETE FROM file_observation_paths WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| {
                RetentionError::new("delete_file_observation_paths", error.to_string())
            })?;
        transaction
            .execute(
                "DELETE FROM file_path_set_chunk_refs WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| {
                RetentionError::new("delete_file_path_set_chunk_refs", error.to_string())
            })?;
        transaction
            .execute(
                "DELETE FROM file_path_set_chunks WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| {
                RetentionError::new("delete_file_path_set_chunks", error.to_string())
            })?;
        transaction
            .execute(
                "DELETE FROM file_path_sets WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_file_path_sets", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM file_paths WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_file_paths", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM semantic_actions WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_semantic_actions", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM diagnostics WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_diagnostics", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM memberships WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_memberships", error.to_string()))?;
        transaction
            .execute(
                "DELETE FROM traces WHERE trace_id = ?1",
                params![trace_id.get()],
            )
            .map_err(|error| RetentionError::new("delete_trace", error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| RetentionError::new("commit_purge", error.to_string()))
    }
}

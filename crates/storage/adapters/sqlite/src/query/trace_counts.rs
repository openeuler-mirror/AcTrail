//! Count one trace through existing indexes without hydrating events or actions.
use model_core::ids::TraceId;
use rusqlite::OptionalExtension;
use storage_core::{StorageError, TraceCounts};

use crate::SqliteStorage;

impl SqliteStorage {
    pub(crate) fn read_trace_counts(&self, trace_id: TraceId) -> Result<TraceCounts, StorageError> {
        self.connection().borrow().query_row(
            "SELECT
                (SELECT COUNT(*) FROM events WHERE trace_id=?1)
                  + (SELECT COUNT(*) FROM event_record_pending WHERE trace_id=?1)
                  + (SELECT COALESCE(SUM(event_count),0) FROM event_record_blocks WHERE trace_id=?1),
                (SELECT COUNT(*) FROM payload_segments WHERE trace_id=?1),
                (SELECT COUNT(*) FROM semantic_actions WHERE trace_id=?1),
                (SELECT COUNT(*) FROM memberships WHERE trace_id=?1)
             FROM traces WHERE trace_id=?1
               AND NOT EXISTS(SELECT 1 FROM tombstones WHERE trace_id=?1)",
            [trace_id.get()],
            |row| Ok(TraceCounts { events: row.get(0)?, payloads: row.get(1)?,
                actions: row.get(2)?, processes: row.get(3)? }),
        ).optional().map_err(|error| StorageError::new("trace_counts", error.to_string()))?
            .ok_or_else(|| StorageError::new("trace_counts", "trace not found or purged"))
    }
}

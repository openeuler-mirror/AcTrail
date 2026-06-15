//! Query-side mapping from rows to storage-contract results.

use rusqlite::params;
use store_read_contract::ReadError;
use store_read_contract::diagnostics::DiagnosticReadStore;
use store_read_contract::events::EventReadStore;
use store_read_contract::filters::TraceFilter;
use store_read_contract::payloads::{PayloadReadStore, PayloadRowLimit, PayloadSegmentQuery};
use store_read_contract::traces::TraceReadStore;
use store_snapshot_contract::SnapshotError;
use store_snapshot_contract::lease::{ExportLease, SnapshotLeaseStore};
use store_snapshot_contract::view::{SnapshotStore, SnapshotView};

use crate::SqliteStorage;
use crate::records::{
    decode_trace_health, diagnostic_from_row, event_from_row, membership_from_row,
    payload_segment_from_row, trace_from_row,
};

impl TraceReadStore for SqliteStorage {
    fn get_trace(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<Option<model_core::trace::TraceRecord>, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new("get_trace", "trace has been purged"));
        }
        let connection = self.connection().borrow();
        read_trace_row(&connection, trace_id)
            .optional()
            .map_err(|error| ReadError::new("query_trace", error.to_string()))
    }

    fn list_traces(
        &self,
        filter: &TraceFilter,
    ) -> Result<Vec<model_core::trace::TraceRecord>, ReadError> {
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare("SELECT * FROM traces ORDER BY created_at ASC")
            .map_err(|error| ReadError::new("prepare_trace_list", error.to_string()))?;
        let rows = statement
            .query_map([], trace_from_row)
            .map_err(|error| ReadError::new("query_trace_list", error.to_string()))?;
        let mut traces = Vec::new();
        for row in rows {
            let trace = row.map_err(|error| ReadError::new("map_trace", error.to_string()))?;
            if !self.is_purged(trace.trace_id) && matches_filter(&trace, filter) {
                traces.push(trace);
            }
        }
        Ok(traces)
    }
}

impl EventReadStore for SqliteStorage {
    fn list_events(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<Vec<model_core::event::DomainEvent>, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new("list_events", "trace has been purged"));
        }
        let connection = self.connection().borrow();
        read_events(&connection, trace_id)
            .map_err(|error| ReadError::new(error.stage, error.message))
    }
}

impl PayloadReadStore for SqliteStorage {
    fn list_payload_segments(
        &self,
        trace_id: model_core::ids::TraceId,
        query: PayloadSegmentQuery,
    ) -> Result<Vec<model_core::payload::PayloadSegment>, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new(
                "list_payload_segments",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        read_payload_segments(&connection, trace_id, query)
            .map_err(|error| ReadError::new(error.stage, error.message))
    }

    fn retained_payload_bytes(&self, trace_id: model_core::ids::TraceId) -> Result<u64, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new(
                "retained_payload_bytes",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        let bytes = connection
            .query_row(
                "SELECT COALESCE(SUM(captured_size), 0) FROM payload_segments WHERE trace_id = ?1",
                params![trace_id.get()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| ReadError::new("retained_payload_bytes", error.to_string()))?;
        u64::try_from(bytes)
            .map_err(|error| ReadError::new("retained_payload_bytes", error.to_string()))
    }
}

impl DiagnosticReadStore for SqliteStorage {
    fn list_diagnostics(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<Vec<model_core::diagnostics::DiagnosticRecord>, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new("list_diagnostics", "trace has been purged"));
        }
        let connection = self.connection().borrow();
        read_diagnostics(&connection, trace_id)
            .map_err(|error| ReadError::new(error.stage, error.message))
    }
}

impl SnapshotLeaseStore for SqliteStorage {
    fn acquire_export_lease(
        &mut self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<ExportLease, SnapshotError> {
        if self.is_purged(trace_id) {
            return Err(SnapshotError::new("acquire_lease", "trace has been purged"));
        }
        let mut leases = self.export_leases().borrow_mut();
        if !leases.insert(trace_id) {
            return Err(SnapshotError::new(
                "acquire_lease",
                "export lease already held",
            ));
        }
        Ok(ExportLease {
            trace_id,
            granted_at: std::time::SystemTime::now(),
        })
    }

    fn release_export_lease(&mut self, lease: ExportLease) -> Result<(), SnapshotError> {
        let removed = self.export_leases().borrow_mut().remove(&lease.trace_id);
        if removed {
            Ok(())
        } else {
            Err(SnapshotError::new("release_lease", "export lease not held"))
        }
    }
}

impl SnapshotStore for SqliteStorage {
    fn read_snapshot(&self, lease: &ExportLease) -> Result<SnapshotView, SnapshotError> {
        if self.is_purged(lease.trace_id) {
            return Err(SnapshotError::new("snapshot", "trace has been purged"));
        }
        let connection = self.connection().borrow();
        let trace = read_trace_row(&connection, lease.trace_id)
            .optional()
            .map_err(|error| SnapshotError::new("query_trace", error.to_string()))?
            .ok_or_else(|| SnapshotError::new("snapshot", "trace not found"))?;
        let memberships = read_memberships(&connection, lease.trace_id)?;
        let events = read_events(&connection, lease.trace_id)?;
        let payload_segments = read_payload_segments(
            &connection,
            lease.trace_id,
            PayloadSegmentQuery {
                segment_id: None,
                direction: None,
                limit: None,
                include_bytes: true,
            },
        )?;
        let diagnostics = read_diagnostics(&connection, lease.trace_id)?;
        Ok(SnapshotView {
            trace,
            memberships,
            events,
            payload_segments,
            diagnostics,
        })
    }
}

impl SqliteStorage {
    pub fn trace_memberships(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<Vec<model_core::process::ProcessMembership>, SnapshotError> {
        if self.is_purged(trace_id) {
            return Err(SnapshotError::new("memberships", "trace has been purged"));
        }
        read_memberships(&self.connection().borrow(), trace_id)
    }

    pub fn count_events_by_variant(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<std::collections::BTreeMap<String, usize>, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new(
                "count_events_by_variant",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare(
                "SELECT payload_variant, COUNT(*) AS count
                 FROM events
                 WHERE trace_id = ?1
                 GROUP BY payload_variant",
            )
            .map_err(|error| ReadError::new("prepare_event_variant_counts", error.to_string()))?;
        let rows = statement
            .query_map(params![trace_id.get()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| ReadError::new("query_event_variant_counts", error.to_string()))?;
        let mut counts = std::collections::BTreeMap::new();
        for row in rows {
            let (variant, count) =
                row.map_err(|error| ReadError::new("map_event_variant_counts", error.to_string()))?;
            counts.insert(
                variant,
                usize::try_from(count)
                    .map_err(|error| ReadError::new("event_variant_count", error.to_string()))?,
            );
        }
        Ok(counts)
    }

    pub fn count_payload_segments(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<usize, ReadError> {
        if self.is_purged(trace_id) {
            return Err(ReadError::new(
                "count_payload_segments",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM payload_segments WHERE trace_id = ?1",
                params![trace_id.get()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| ReadError::new("count_payload_segments", error.to_string()))?;
        usize::try_from(count)
            .map_err(|error| ReadError::new("count_payload_segments", error.to_string()))
    }

    pub(crate) fn is_purged(&self, trace_id: model_core::ids::TraceId) -> bool {
        let connection = self.connection().borrow();
        connection
            .query_row(
                "SELECT health FROM tombstones WHERE trace_id = ?1",
                params![trace_id.get()],
                |row| row.get::<_, String>(0),
            )
            .map(|health| decode_trace_health(&health).is_ok())
            .unwrap_or(false)
    }
}

fn read_trace_row(
    connection: &rusqlite::Connection,
    trace_id: model_core::ids::TraceId,
) -> Result<model_core::trace::TraceRecord, rusqlite::Error> {
    let mut statement = connection.prepare("SELECT * FROM traces WHERE trace_id = ?1")?;
    statement.query_row(params![trace_id.get()], trace_from_row)
}

fn read_memberships(
    connection: &rusqlite::Connection,
    trace_id: model_core::ids::TraceId,
) -> Result<Vec<model_core::process::ProcessMembership>, SnapshotError> {
    let mut statement = connection
        .prepare(
            "SELECT * FROM memberships WHERE trace_id = ?1 ORDER BY pid ASC, start_ticks ASC, generation ASC",
        )
        .map_err(|error| SnapshotError::new("prepare_memberships", error.to_string()))?;
    let rows = statement
        .query_map(params![trace_id.get()], membership_from_row)
        .map_err(|error| SnapshotError::new("query_memberships", error.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::new("map_memberships", error.to_string()))
}

fn read_events(
    connection: &rusqlite::Connection,
    trace_id: model_core::ids::TraceId,
) -> Result<Vec<model_core::event::DomainEvent>, SnapshotError> {
    let mut statement = connection
        .prepare("SELECT * FROM events WHERE trace_id = ?1 ORDER BY observed_at ASC, event_id ASC")
        .map_err(|error| SnapshotError::new("prepare_events", error.to_string()))?;
    let rows = statement
        .query_map(params![trace_id.get()], event_from_row)
        .map_err(|error| SnapshotError::new("query_events", error.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::new("map_events", error.to_string()))
}

fn read_payload_segments(
    connection: &rusqlite::Connection,
    trace_id: model_core::ids::TraceId,
    query: PayloadSegmentQuery,
) -> Result<Vec<model_core::payload::PayloadSegment>, SnapshotError> {
    let direction = query
        .direction
        .map(crate::records::encode_payload_direction);
    let segment_id = query.segment_id.map(|value| value.get());
    let mut statement = connection
        .prepare(
            "SELECT * FROM payload_segments
             WHERE trace_id = ?1
               AND (?2 IS NULL OR segment_id = ?2)
               AND (?3 IS NULL OR direction = ?3)
             ORDER BY observed_at ASC, segment_id ASC",
        )
        .map_err(|error| SnapshotError::new("prepare_payload_segments", error.to_string()))?;
    let rows = statement
        .query_map(
            rusqlite::params![trace_id.get(), segment_id, direction],
            |row| payload_segment_from_row(row),
        )
        .map_err(|error| SnapshotError::new("query_payload_segments", error.to_string()))?;
    let mut segments = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::new("map_payload_segments", error.to_string()))?;
    if !query.include_bytes {
        for segment in &mut segments {
            segment.bytes.clear();
        }
    }
    Ok(limit_payload_segments(segments, query.limit))
}

fn limit_payload_segments(
    mut segments: Vec<model_core::payload::PayloadSegment>,
    limit: Option<PayloadRowLimit>,
) -> Vec<model_core::payload::PayloadSegment> {
    match limit {
        Some(PayloadRowLimit::Head(count)) => {
            segments.truncate(count);
            segments
        }
        Some(PayloadRowLimit::Tail(count)) if segments.len() > count => {
            segments.split_off(segments.len() - count)
        }
        Some(PayloadRowLimit::Tail(_)) | None => segments,
    }
}

fn read_diagnostics(
    connection: &rusqlite::Connection,
    trace_id: model_core::ids::TraceId,
) -> Result<Vec<model_core::diagnostics::DiagnosticRecord>, SnapshotError> {
    let mut statement = connection
        .prepare(
            "SELECT * FROM diagnostics WHERE trace_id = ?1 OR trace_id IS NULL ORDER BY emitted_at ASC, diagnostic_id ASC",
        )
        .map_err(|error| SnapshotError::new("prepare_diagnostics", error.to_string()))?;
    let rows = statement
        .query_map(params![trace_id.get()], diagnostic_from_row)
        .map_err(|error| SnapshotError::new("query_diagnostics", error.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::new("map_diagnostics", error.to_string()))
}

fn matches_filter(trace: &model_core::trace::TraceRecord, filter: &TraceFilter) -> bool {
    (filter.trace_ids.is_empty() || filter.trace_ids.contains(&trace.trace_id))
        && (filter.root_pids.is_empty()
            || filter.root_pids.contains(&trace.root_process_identity.pid))
        && (filter.tags.is_empty() || filter.tags.iter().all(|tag| trace.tags.contains(tag)))
        && (filter.names.is_empty() || filter.names.contains(&trace.display_name))
}

trait OptionalRow<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalRow<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

//! Query-side mapping from rows to storage-contract results.

use std::collections::{BTreeMap, HashMap};

use rusqlite::params;
use store_read_contract::ReadError;
use store_read_contract::diagnostics::DiagnosticReadStore;
use store_read_contract::events::EventReadStore;
use store_read_contract::filters::TraceFilter;
use store_read_contract::payloads::{PayloadReadStore, PayloadRowLimit, PayloadSegmentQuery};
use store_read_contract::traces::TraceReadStore;
use store_snapshot_contract::SnapshotError;
use store_snapshot_contract::lease::{
    SnapshotLeaseStore, TraceLease, TraceLeasePurpose, TraceLeaseToken,
};
use store_snapshot_contract::view::{SnapshotStore, SnapshotView};

use crate::SqliteStorage;
use crate::config::{
    SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS, SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES,
};
use crate::records::{
    PAYLOAD_DIRECTION_MASK, decode_event_kind, decode_event_record_block,
    decode_event_record_block_kind_counts, decode_event_record_frame, decode_trace_health,
    diagnostic_from_row, event_from_row, event_kind_name, membership_from_row,
    payload_segment_from_row, trace_from_row,
};

pub(crate) struct TraceLeaseRegistry {
    next_token: u64,
    leases: BTreeMap<TraceLeaseToken, (model_core::ids::TraceId, TraceLeasePurpose)>,
}

impl TraceLeaseRegistry {
    pub(crate) const fn new() -> Self {
        Self {
            next_token: 1,
            leases: BTreeMap::new(),
        }
    }

    fn acquire(
        &mut self,
        trace_id: model_core::ids::TraceId,
        purpose: TraceLeasePurpose,
    ) -> Result<TraceLeaseToken, SnapshotError> {
        let token = TraceLeaseToken::new(self.next_token);
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or_else(|| SnapshotError::new("acquire_lease", "trace lease token exhausted"))?;
        self.leases.insert(token, (trace_id, purpose));
        Ok(token)
    }

    fn release(&mut self, lease: &TraceLease) -> bool {
        if !self.contains(lease) {
            return false;
        }
        self.leases.remove(&lease.token).is_some()
    }

    pub(crate) fn protects(&self, trace_id: model_core::ids::TraceId) -> bool {
        self.leases
            .values()
            .any(|(leased_trace_id, _)| *leased_trace_id == trace_id)
    }

    fn contains(&self, lease: &TraceLease) -> bool {
        self.leases
            .get(&lease.token)
            .is_some_and(|identity| *identity == (lease.trace_id, lease.purpose))
    }
}

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
                "SELECT COALESCE(SUM(length(bytes)), 0) FROM payload_segments WHERE trace_id = ?1",
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
    fn acquire_trace_lease(
        &mut self,
        trace_id: model_core::ids::TraceId,
        purpose: TraceLeasePurpose,
    ) -> Result<TraceLease, SnapshotError> {
        if self.is_purged(trace_id) {
            return Err(SnapshotError::new("acquire_lease", "trace has been purged"));
        }
        let token = self
            .trace_leases()
            .borrow_mut()
            .acquire(trace_id, purpose)?;
        Ok(TraceLease {
            token,
            trace_id,
            purpose,
            granted_at: std::time::SystemTime::now(),
        })
    }

    fn release_trace_lease(&mut self, lease: TraceLease) -> Result<(), SnapshotError> {
        let removed = self.trace_leases().borrow_mut().release(&lease);
        if removed {
            Ok(())
        } else {
            Err(SnapshotError::new(
                "release_lease",
                "trace lease token or identity is not active",
            ))
        }
    }
}

impl SnapshotStore for SqliteStorage {
    fn read_snapshot(&self, lease: &TraceLease) -> Result<SnapshotView, SnapshotError> {
        if lease.purpose != TraceLeasePurpose::Export {
            return Err(SnapshotError::new(
                "snapshot",
                "snapshot reads require an export-purpose trace lease",
            ));
        }
        if !self.trace_leases().borrow().contains(lease) {
            return Err(SnapshotError::new("snapshot", "trace lease is not active"));
        }
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
                "SELECT kind_code, COUNT(*) AS count
                 FROM events
                 WHERE trace_id = ?1
                 GROUP BY kind_code",
            )
            .map_err(|error| ReadError::new("prepare_event_variant_counts", error.to_string()))?;
        let rows = statement
            .query_map(params![trace_id.get()], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| ReadError::new("query_event_variant_counts", error.to_string()))?;
        let mut counts = std::collections::BTreeMap::new();
        for row in rows {
            let (kind_code, count) =
                row.map_err(|error| ReadError::new("map_event_variant_counts", error.to_string()))?;
            let variant = event_kind_name(
                decode_event_kind(kind_code)
                    .map_err(|error| ReadError::new("decode_event_variant", error.to_string()))?,
            )
            .to_string();
            counts.insert(
                variant,
                usize::try_from(count)
                    .map_err(|error| ReadError::new("event_variant_count", error.to_string()))?,
            );
        }
        let mut statement = connection
            .prepare("SELECT kind_counts FROM event_record_blocks WHERE trace_id = ?1")
            .map_err(|error| {
                ReadError::new("prepare_event_block_variant_counts", error.to_string())
            })?;
        let rows = statement
            .query_map(params![trace_id.get()], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|error| {
                ReadError::new("query_event_block_variant_counts", error.to_string())
            })?;
        for row in rows {
            let encoded = row.map_err(|error| {
                ReadError::new("map_event_block_variant_counts", error.to_string())
            })?;
            let block_counts =
                decode_event_record_block_kind_counts(&encoded).map_err(|error| {
                    ReadError::new("decode_event_block_variant_counts", error.to_string())
                })?;
            for (kind_code, count) in block_counts.into_iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let variant =
                    event_kind_name(decode_event_kind(kind_code as i64).map_err(|error| {
                        ReadError::new("decode_event_block_variant", error.to_string())
                    })?)
                    .to_string();
                let count = usize::try_from(count).map_err(|error| {
                    ReadError::new("event_block_variant_count", error.to_string())
                })?;
                let total = counts.entry(variant).or_default();
                *total = total.saturating_add(count);
            }
        }
        let mut statement = connection
            .prepare(
                "SELECT kind_code, COUNT(*) AS count
                 FROM event_record_pending
                 WHERE trace_id = ?1
                 GROUP BY kind_code",
            )
            .map_err(|error| {
                ReadError::new("prepare_pending_event_variant_counts", error.to_string())
            })?;
        let rows = statement
            .query_map(params![trace_id.get()], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| {
                ReadError::new("query_pending_event_variant_counts", error.to_string())
            })?;
        for row in rows {
            let (kind_code, count) = row.map_err(|error| {
                ReadError::new("map_pending_event_variant_counts", error.to_string())
            })?;
            let variant = event_kind_name(decode_event_kind(kind_code).map_err(|error| {
                ReadError::new("decode_pending_event_variant", error.to_string())
            })?)
            .to_string();
            let count = usize::try_from(count).map_err(|error| {
                ReadError::new("pending_event_variant_count", error.to_string())
            })?;
            let total = counts.entry(variant).or_default();
            *total = total.saturating_add(count);
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
        .prepare("SELECT * FROM memberships WHERE trace_id = ?1 ORDER BY process_id ASC")
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
        .prepare(
            "SELECT event.*,
                    COALESCE(event.payload_inline, dictionary.payload) AS payload,
                    path.path_text AS payload_path
             FROM events event
             LEFT JOIN event_payload_dictionary dictionary
              ON dictionary.payload_id = event.payload_id
              AND dictionary.trace_id = event.trace_id
             LEFT JOIN file_paths path
               ON path.path_id = event.payload_path_id
              AND path.trace_id = event.trace_id
             WHERE event.trace_id = ?1
             ORDER BY event.observed_at ASC, event.event_id ASC",
        )
        .map_err(|error| SnapshotError::new("prepare_events", error.to_string()))?;
    let rows = statement
        .query_map(params![trace_id.get()], |row| {
            event_from_row(connection, row)
        })
        .map_err(|error| SnapshotError::new("query_events", error.to_string()))?;
    let mut events = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::new("map_events", error.to_string()))?;
    let has_encoded_records = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM event_record_blocks WHERE trace_id = ?1
                UNION ALL
                SELECT 1 FROM event_record_pending WHERE trace_id = ?1
             )",
            params![trace_id.get()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| SnapshotError::new("find_encoded_event_records", error.to_string()))?;
    if !has_encoded_records {
        return Ok(events);
    }
    let mut paths = HashMap::<i64, String>::new();
    let mut path_statement = connection
        .prepare("SELECT path_text FROM file_paths WHERE trace_id = ?1 AND path_id = ?2")
        .map_err(|error| SnapshotError::new("prepare_event_path", error.to_string()))?;
    let mut resolve_path = |path_id: i64| -> Result<String, rusqlite::Error> {
        if let Some(path) = paths.get(&path_id) {
            return Ok(path.clone());
        }
        let path = path_statement.query_row(params![trace_id.get(), path_id], |row| {
            row.get::<_, String>(0)
        })?;
        paths.insert(path_id, path.clone());
        Ok(path)
    };
    let mut statement = connection
        .prepare(
            "SELECT codec_version, event_count, uncompressed_bytes,
                    length(encoded_bytes), encoded_bytes
             FROM event_record_blocks
             WHERE trace_id = ?1
             ORDER BY min_observed_at ASC, first_event_id ASC",
        )
        .map_err(|error| SnapshotError::new("prepare_event_record_blocks", error.to_string()))?;
    let mut rows = statement
        .query(params![trace_id.get()])
        .map_err(|error| SnapshotError::new("query_event_record_blocks", error.to_string()))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| SnapshotError::new("read_event_record_block", error.to_string()))?
    {
        let codec_version = row
            .get::<_, i64>(0)
            .map_err(|error| SnapshotError::new("read_event_record_block", error.to_string()))?;
        let event_count = row
            .get::<_, usize>(1)
            .map_err(|error| SnapshotError::new("read_event_record_block", error.to_string()))?;
        let uncompressed_bytes = row
            .get::<_, usize>(2)
            .map_err(|error| SnapshotError::new("read_event_record_block", error.to_string()))?;
        let encoded_length = row
            .get::<_, usize>(3)
            .map_err(|error| SnapshotError::new("read_event_record_block", error.to_string()))?;
        if event_count == 0
            || event_count > SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS
            || uncompressed_bytes == 0
            || uncompressed_bytes > SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES
        {
            return Err(SnapshotError::new(
                "read_event_record_block",
                "block metadata exceeds the codec safety boundary",
            ));
        }
        if encoded_length > zstd::zstd_safe::compress_bound(uncompressed_bytes) {
            return Err(SnapshotError::new(
                "read_event_record_block",
                "compressed block exceeds the codec bound",
            ));
        }
        let encoded = row
            .get::<_, Vec<u8>>(4)
            .map_err(|error| SnapshotError::new("read_event_record_block", error.to_string()))?;
        if encoded.len() != encoded_length {
            return Err(SnapshotError::new(
                "read_event_record_block",
                "compressed block length changed while reading",
            ));
        }
        events.extend(
            decode_event_record_block(
                trace_id.get(),
                codec_version,
                event_count,
                uncompressed_bytes,
                &encoded,
                &mut resolve_path,
            )
            .map_err(|error| SnapshotError::new("decode_event_record_block", error.to_string()))?,
        );
    }
    drop(rows);
    drop(statement);
    let mut statement = connection
        .prepare(
            "SELECT length(encoded_frame), encoded_frame
             FROM event_record_pending
             WHERE trace_id = ?1
             ORDER BY event_id ASC",
        )
        .map_err(|error| SnapshotError::new("prepare_pending_events", error.to_string()))?;
    let mut rows = statement
        .query(params![trace_id.get()])
        .map_err(|error| SnapshotError::new("query_pending_events", error.to_string()))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| SnapshotError::new("read_pending_event", error.to_string()))?
    {
        let encoded_length = row
            .get::<_, usize>(0)
            .map_err(|error| SnapshotError::new("read_pending_event", error.to_string()))?;
        if encoded_length == 0 || encoded_length > SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES
        {
            return Err(SnapshotError::new(
                "read_pending_event",
                "pending event exceeds the codec safety boundary",
            ));
        }
        let encoded = row
            .get::<_, Vec<u8>>(1)
            .map_err(|error| SnapshotError::new("read_pending_event", error.to_string()))?;
        if encoded.len() != encoded_length {
            return Err(SnapshotError::new(
                "read_pending_event",
                "pending event length changed while reading",
            ));
        }
        events.push(
            decode_event_record_frame(trace_id.get(), &encoded, &mut resolve_path)
                .map_err(|error| SnapshotError::new("decode_pending_event", error.to_string()))?,
        );
    }
    events.sort_by(|left, right| {
        left.envelope
            .observed_at
            .cmp(&right.envelope.observed_at)
            .then_with(|| {
                left.envelope
                    .event_id
                    .get()
                    .cmp(&right.envelope.event_id.get())
            })
    });
    Ok(events)
}

fn read_payload_segments(
    connection: &rusqlite::Connection,
    trace_id: model_core::ids::TraceId,
    query: PayloadSegmentQuery,
) -> Result<Vec<model_core::payload::PayloadSegment>, SnapshotError> {
    let direction = query
        .direction
        .map(crate::records::PayloadSegmentMeta::direction_bits);
    let segment_id = query.segment_id.map(|value| value.get());
    let (order_direction, row_limit, reverse_rows) = payload_segment_query_limit(query.limit)?;
    let body_projection = if query.include_bytes {
        "bytes"
    } else {
        "X'' AS bytes"
    };
    let sql = format!(
        "SELECT segment_id, trace_id, observed_at, process_id, segment_meta,
                stream_key, sequence, original_size, captured_size, operation_id,
                operation_offset, operation_original_size, operation_captured_size,
                library, symbol, protocol_hint, storage_omission, {body_projection}
         FROM payload_segments
         WHERE trace_id = ?1
           AND (?2 IS NULL OR segment_id = ?2)
           AND (?3 IS NULL OR (segment_meta & {PAYLOAD_DIRECTION_MASK}) = ?3)
         ORDER BY observed_at {order_direction}, segment_id {order_direction}
         LIMIT ?4"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| SnapshotError::new("prepare_payload_segments", error.to_string()))?;
    let rows = statement
        .query_map(
            rusqlite::params![trace_id.get(), segment_id, direction, row_limit],
            |row| payload_segment_from_row(row),
        )
        .map_err(|error| SnapshotError::new("query_payload_segments", error.to_string()))?;
    let mut segments = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| SnapshotError::new("map_payload_segments", error.to_string()))?;
    if reverse_rows {
        segments.reverse();
    }
    Ok(segments)
}

fn payload_segment_query_limit(
    limit: Option<PayloadRowLimit>,
) -> Result<(&'static str, i64, bool), SnapshotError> {
    match limit {
        Some(PayloadRowLimit::Head(count)) => Ok(("ASC", payload_row_limit_to_i64(count)?, false)),
        Some(PayloadRowLimit::Tail(count)) => Ok(("DESC", payload_row_limit_to_i64(count)?, true)),
        None => Ok(("ASC", -1, false)),
    }
}

fn payload_row_limit_to_i64(count: usize) -> Result<i64, SnapshotError> {
    i64::try_from(count)
        .map_err(|error| SnapshotError::new("payload_segment_limit", error.to_string()))
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
        && filter.root_pids.is_empty()
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

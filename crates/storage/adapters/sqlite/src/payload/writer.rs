//! Payload storage policy runs after analysis and never changes capture facts.

use model_core::payload::PayloadSegment;
use rusqlite::{Connection, OptionalExtension, params};
use store_write_contract::{WriteError, payloads::PayloadWriteStore};

use crate::SqliteStorage;
use crate::records::{PayloadSegmentMeta, encode_time};

use super::retention::PayloadRetentionState;

impl PayloadWriteStore for SqliteStorage {
    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), WriteError> {
        let connection = self.connection().borrow();
        let mut retention = self.payload_retention.borrow_mut();
        let standalone = connection.is_autocommit();
        if standalone {
            connection
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|error| WriteError::new("begin_payload_append", error.to_string()))?;
        }
        let result = retention.write_segment(&connection, &segment);
        let result = if standalone {
            match result {
                Ok(()) => connection.execute_batch("COMMIT"),
                Err(error) => Err(error),
            }
        } else {
            result
        };
        if result.is_err() {
            retention.invalidate();
            if standalone {
                if let Err(rollback) = connection.execute_batch("ROLLBACK") {
                    return Err(WriteError::new(
                        "rollback_payload_append",
                        format!(
                            "write failed: {}; rollback failed: {rollback}",
                            result.unwrap_err()
                        ),
                    ));
                }
            }
        }
        result.map_err(|error| WriteError::new("append_payload_segment", error.to_string()))
    }
}

impl PayloadRetentionState {
    fn write_segment(
        &mut self,
        connection: &Connection,
        segment: &PayloadSegment,
    ) -> rusqlite::Result<()> {
        let retained =
            self.retained_bytes(connection, segment.trace_id.get(), segment.source_boundary)?;
        let (body, omission) =
            self.select_body(segment.source_boundary, retained, 0, &segment.bytes)?;
        let meta = PayloadSegmentMeta::from_segment(segment).code();
        let inserted = connection
            .prepare_cached(
                "INSERT INTO payload_segments (
                segment_id, trace_id, observed_at, process_id, segment_meta,
                stream_key, sequence, original_size, captured_size, operation_id,
                operation_offset, operation_original_size, operation_captured_size,
                library, symbol, protocol_hint, bytes, storage_omission
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
             ON CONFLICT(segment_id) DO NOTHING RETURNING segment_id",
            )?
            .query_row(
                params![
                    segment.segment_id.get(),
                    segment.trace_id.get(),
                    encode_time(segment.observed_at),
                    segment.process.get(),
                    meta,
                    segment.stream_key.as_str(),
                    segment.sequence,
                    segment.original_size,
                    segment.captured_size,
                    segment.operation_id,
                    segment.operation_offset,
                    segment.operation_original_size,
                    segment.operation_captured_size,
                    segment.library,
                    segment.symbol,
                    segment.protocol_hint,
                    body,
                    omission as i64,
                ],
                |_| Ok(()),
            )
            .optional()?;
        if inserted.is_some() {
            return self.record_write(
                segment.trace_id.get(),
                segment.source_boundary,
                retained,
                0,
                body.len(),
            );
        }
        let (trace_id, process_id, source, replaced) = connection
            .prepare_cached(
                "SELECT trace_id, process_id, (segment_meta >> 2) & 3, length(bytes)
             FROM payload_segments WHERE segment_id=?1",
            )?
            .query_row(params![segment.segment_id.get()], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, usize>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            })?;
        if trace_id != segment.trace_id.get()
            || process_id != segment.process.get()
            || source != Self::source_index(segment.source_boundary)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "payload segment identity conflict".into(),
            ));
        }
        let (body, omission) =
            self.select_body(segment.source_boundary, retained, replaced, &segment.bytes)?;
        connection
            .prepare_cached(
                "UPDATE payload_segments SET observed_at=?2, segment_meta=?3,
             stream_key=?4, sequence=?5, original_size=?6, captured_size=?7,
             operation_id=?8, operation_offset=?9, operation_original_size=?10,
             operation_captured_size=?11, library=?12, symbol=?13, protocol_hint=?14,
             bytes=?15, storage_omission=?16 WHERE segment_id=?1",
            )?
            .execute(params![
                segment.segment_id.get(),
                encode_time(segment.observed_at),
                meta,
                segment.stream_key.as_str(),
                segment.sequence,
                segment.original_size,
                segment.captured_size,
                segment.operation_id,
                segment.operation_offset,
                segment.operation_original_size,
                segment.operation_captured_size,
                segment.library,
                segment.symbol,
                segment.protocol_hint,
                body,
                omission as i64,
            ])?;
        self.record_write(
            trace_id,
            segment.source_boundary,
            retained,
            replaced,
            body.len(),
        )
    }
}

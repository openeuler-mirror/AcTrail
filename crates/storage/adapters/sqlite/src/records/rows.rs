//! Row decoders used by SQLite queries and snapshots.

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::{DomainEvent, EventEnvelope};
use model_core::ids::OtelTraceId;
use model_core::payload::{PayloadSegment, PayloadSegmentId, PayloadStreamKey};
use model_core::process::{ExitStatus, ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceAlertToken, TraceRecord, TraceTiming};
use rusqlite::types::Type;
use rusqlite::{Error as SqlError, Row};

use crate::records::{
    BlockKind, EventMeta, PayloadBlock, PayloadSegmentMeta, decode_diagnostic_kind,
    decode_diagnostic_severity, decode_event_kind, decode_event_payload,
    decode_exit_observation_source, decode_map, decode_membership_state, decode_policy_record,
    decode_tags, decode_time, decode_trace_health, decode_trace_lifecycle, i64_to_bool,
    restore_shared_path,
};

pub fn trace_from_row(row: &Row<'_>) -> Result<TraceRecord, SqlError> {
    let otel_trace_id_column = row.as_ref().column_index("otel_trace_id")?;
    let otel_trace_id_bytes = row.get::<_, Vec<u8>>(otel_trace_id_column)?;
    let otel_trace_id = OtelTraceId::from_slice(&otel_trace_id_bytes).ok_or_else(|| {
        SqlError::FromSqlConversionFailure(
            otel_trace_id_column,
            Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "OTLP trace identity must contain exactly 16 non-zero bytes",
            )),
        )
    })?;
    let alert_token_column = row.as_ref().column_index("alert_token")?;
    let alert_token_bytes = row.get::<_, Vec<u8>>(alert_token_column)?;
    let alert_token = TraceAlertToken::from_slice(&alert_token_bytes).ok_or_else(|| {
        SqlError::FromSqlConversionFailure(
            alert_token_column,
            Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "trace alert token must contain exactly 32 bytes",
            )),
        )
    })?;
    Ok(TraceRecord {
        trace_id: model_core::ids::TraceId::new(row.get::<_, u64>("trace_id")?),
        otel_trace_id,
        alert_token,
        root_process_identity: ProcessIdentity::new(row.get("root_process_id")?),
        // Active trace queries are served by `TraceRuntime`, which captures the
        // namespace at attach. Historical SQLite rows predate this projection.
        root_pid_namespace: None,
        root_container_id: row.get::<_, Option<String>>("root_container_id")?,
        // pod uid and host.id are v1 live-export-only (not persisted); reloaded
        // traces carry None. Known gap: the `actrailviewer` storage export of a
        // trace therefore omits the `k8s.pod.uid` and `host.id` resource
        // attributes that its live export carries. Nothing may derive an OTLP
        // trace id from these (see `otel_codec::service::otel_trace_id_u128`) —
        // that would split one trace across the two export paths. A future
        // schema bump persists both together.
        root_pod_uid: None,
        root_host_id: None,
        root_working_directory: row.get::<_, Option<String>>("root_working_directory")?,
        display_name: model_core::ids::TraceName::new(row.get::<_, String>("display_name")?),
        profile_name: model_core::ids::ProfileName::new(row.get::<_, String>("profile_name")?),
        tags: decode_tags(&row.get::<_, String>("tags")?),
        lifecycle_state: decode_trace_lifecycle(&row.get::<_, String>("lifecycle_state")?)?,
        health: decode_trace_health(&row.get::<_, String>("health")?)?,
        timings: TraceTiming {
            created_at: decode_time(row.get("created_at")?),
            started_at: row.get::<_, Option<i64>>("started_at")?.map(decode_time),
            completed_at: row.get::<_, Option<i64>>("completed_at")?.map(decode_time),
            exited_at: row.get::<_, Option<i64>>("exited_at")?.map(decode_time),
            failed_at: row.get::<_, Option<i64>>("failed_at")?.map(decode_time),
        },
    })
}

pub fn membership_from_row(row: &Row<'_>) -> Result<ProcessMembership, SqlError> {
    let exit_source = row
        .get::<_, Option<String>>("exit_observation_source")
        .ok()
        .flatten()
        .map(|raw| decode_exit_observation_source(&raw))
        .transpose()?;
    let exit_status = row
        .get::<_, Option<i64>>("exit_observed_at")?
        .map(|observed_at| ExitStatus {
            code: row.get("exit_code").ok().flatten(),
            observed_at: decode_time(observed_at),
            source: exit_source,
        });

    Ok(ProcessMembership {
        trace_id: model_core::ids::TraceId::new(row.get("trace_id")?),
        identity: ProcessIdentity::new(row.get("process_id")?),
        inherited_from: row
            .get::<_, Option<u64>>("inherited_from_process_id")?
            .map(ProcessIdentity::new),
        observed_at: row.get::<_, Option<i64>>("observed_at")?.map(decode_time),
        capture_enabled: i64_to_bool(row.get("capture_enabled")?),
        propagation_enabled: i64_to_bool(row.get("propagation_enabled")?),
        state: decode_membership_state(&row.get::<_, String>("membership_state")?)?,
        exit_status,
    })
}

pub fn event_from_row(
    connection: &rusqlite::Connection,
    row: &Row<'_>,
) -> Result<DomainEvent, SqlError> {
    let event_meta = EventMeta::decode(row.get("event_meta")?)?;
    let envelope = EventEnvelope {
        event_id: model_core::ids::EventId::new(row.get("event_id")?),
        trace_id: model_core::ids::TraceId::new(row.get("trace_id")?),
        observed_at: decode_time(row.get("observed_at")?),
        process: ProcessIdentity::new(row.get("process_id")?),
        collector: event_meta.collector()?,
        kind: decode_event_kind(row.get("kind_code")?)?,
        flags: event_meta.flags()?,
    };
    let fields = row.get::<_, Vec<u8>>("payload")?;
    let blocks = if event_meta.has_payload_blocks() {
        read_payload_blocks(connection, envelope.event_id.get())?
    } else {
        Vec::new()
    };
    let mut payload = decode_event_payload(&fields, &blocks)?;
    let payload_path_id = row.get::<_, Option<i64>>("payload_path_id")?;
    let payload_path = row.get::<_, Option<String>>("payload_path")?;
    if payload_path_id.is_some() != payload_path.is_some() {
        return Err(SqlError::InvalidQuery);
    }
    restore_shared_path(&mut payload, payload_path)?;
    let (note, redactions, truncations) = if event_meta.has_policy_details() {
        connection.query_row(
            "SELECT note, redactions, truncations
             FROM event_policy_details WHERE event_id = ?1",
            [envelope.event_id.get()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?
    } else {
        (None, String::new(), String::new())
    };
    let policy = decode_policy_record(event_meta.policy()?, note, &redactions, &truncations)?;
    Ok(DomainEvent {
        envelope,
        payload,
        policy,
    })
}

fn read_payload_blocks(
    connection: &rusqlite::Connection,
    event_id: u64,
) -> Result<Vec<PayloadBlock>, SqlError> {
    let mut statement = connection.prepare(
        "SELECT kind, encoded_bytes FROM event_payload_blocks
         WHERE event_id = ?1 ORDER BY block_order ASC",
    )?;
    let rows = statement.query_map([event_id], |row| {
        let kind = BlockKind::from_i64(row.get::<_, i64>("kind")?).ok_or(SqlError::InvalidQuery)?;
        let encoded_bytes = row.get::<_, Vec<u8>>("encoded_bytes")?;
        let bytes = zstd::stream::decode_all(std::io::Cursor::new(encoded_bytes))
            .map_err(|_| SqlError::InvalidQuery)?;
        Ok(PayloadBlock { kind, bytes })
    })?;
    let blocks = rows.collect::<Result<Vec<_>, _>>()?;
    if blocks.is_empty() {
        return Err(SqlError::InvalidQuery);
    }
    Ok(blocks)
}

pub fn payload_segment_from_row(row: &Row<'_>) -> Result<PayloadSegment, SqlError> {
    let segment_meta = PayloadSegmentMeta::from_code(row.get("segment_meta")?)?;
    Ok(PayloadSegment {
        segment_id: PayloadSegmentId::new(row.get("segment_id")?),
        trace_id: model_core::ids::TraceId::new(row.get("trace_id")?),
        observed_at: decode_time(row.get("observed_at")?),
        process: ProcessIdentity::new(row.get("process_id")?),
        source_boundary: segment_meta.source_boundary()?,
        content_state: segment_meta.content_state()?,
        direction: segment_meta.direction()?,
        stream_key: PayloadStreamKey::new(row.get::<_, String>("stream_key")?),
        sequence: row.get("sequence")?,
        original_size: row.get("original_size")?,
        captured_size: row.get("captured_size")?,
        operation_id: row.get("operation_id")?,
        operation_offset: row.get("operation_offset")?,
        operation_original_size: row.get("operation_original_size")?,
        operation_captured_size: row.get("operation_captured_size")?,
        operation_completion_state: segment_meta.operation_completion_state()?,
        truncation: segment_meta.truncation()?,
        redaction: segment_meta.redaction()?,
        library: row.get("library")?,
        symbol: row.get("symbol")?,
        protocol_hint: row.get("protocol_hint")?,
        bytes: row.get("bytes")?,
    })
}

pub fn diagnostic_from_row(row: &Row<'_>) -> Result<DiagnosticRecord, SqlError> {
    Ok(DiagnosticRecord {
        diagnostic_id: model_core::ids::DiagnosticId::new(row.get("diagnostic_id")?),
        trace_id: row
            .get::<_, Option<u64>>("trace_id")?
            .map(model_core::ids::TraceId::new),
        process: row
            .get::<_, Option<u64>>("process_id")?
            .map(ProcessIdentity::new),
        kind: decode_diagnostic_kind(&row.get::<_, String>("kind")?)?,
        severity: decode_diagnostic_severity(&row.get::<_, String>("severity")?)?,
        emitted_at: decode_time(row.get("emitted_at")?),
        message: row.get("message")?,
        metadata: decode_map(&row.get::<_, String>("metadata")?),
    })
}

//! Row decoders used by SQLite queries and snapshots.

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::{DomainEvent, EventEnvelope, EventFlags};
use model_core::payload::{PayloadSegment, PayloadSegmentId, PayloadStreamKey};
use model_core::process::{ExitStatus, NamespaceIdentity, ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceRecord, TraceTiming};
use rusqlite::{Error as SqlError, Row};

use crate::records::{
    decode_diagnostic_kind, decode_diagnostic_severity, decode_event_kind, decode_event_payload,
    decode_exit_observation_source, decode_map, decode_membership_state,
    decode_payload_content_state, decode_payload_direction,
    decode_payload_operation_completion_state, decode_payload_redaction_state,
    decode_payload_source_boundary, decode_payload_truncation_state, decode_policy_record,
    decode_tags, decode_time, decode_trace_health, decode_trace_lifecycle, i64_to_bool,
};

pub fn trace_from_row(row: &Row<'_>) -> Result<TraceRecord, SqlError> {
    Ok(TraceRecord {
        trace_id: model_core::ids::TraceId::new(row.get::<_, u64>("trace_id")?),
        root_process_identity: ProcessIdentity {
            pid: row.get("root_pid")?,
            task_id: row.get("root_task_id")?,
            start_time_ticks: row.get("root_start_ticks")?,
            pid_namespace: row
                .get::<_, Option<String>>("root_pid_namespace")?
                .map(NamespaceIdentity::new),
            generation: row.get("root_generation")?,
        },
        display_name: model_core::ids::TraceName::new(row.get::<_, String>("display_name")?),
        profile_name: model_core::ids::ProfileName::new(row.get::<_, String>("profile_name")?),
        tags: decode_tags(&row.get::<_, String>("tags")?),
        lifecycle_state: decode_trace_lifecycle(&row.get::<_, String>("lifecycle_state")?)?,
        health: decode_trace_health(&row.get::<_, String>("health")?)?,
        timings: TraceTiming {
            created_at: decode_time(row.get("created_at")?),
            started_at: row.get::<_, Option<i64>>("started_at")?.map(decode_time),
            completed_at: row.get::<_, Option<i64>>("completed_at")?.map(decode_time),
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
        identity: ProcessIdentity {
            pid: row.get("pid")?,
            task_id: row.get("task_id")?,
            start_time_ticks: row.get("start_ticks")?,
            pid_namespace: row
                .get::<_, Option<String>>("pid_namespace")?
                .map(NamespaceIdentity::new),
            generation: row.get("generation")?,
        },
        inherited_from: row.get::<_, Option<u32>>("inherited_from_pid")?.map(|pid| {
            ProcessIdentity {
                pid,
                task_id: row.get("inherited_from_task_id").ok().flatten(),
                start_time_ticks: row.get("inherited_from_start_ticks").unwrap_or_default(),
                pid_namespace: row
                    .get::<_, Option<String>>("inherited_from_pid_namespace")
                    .ok()
                    .flatten()
                    .map(NamespaceIdentity::new),
                generation: row.get("inherited_from_generation").unwrap_or_default(),
            }
        }),
        observed_at: row.get::<_, Option<i64>>("observed_at")?.map(decode_time),
        capture_enabled: i64_to_bool(row.get("capture_enabled")?),
        propagation_enabled: i64_to_bool(row.get("propagation_enabled")?),
        state: decode_membership_state(&row.get::<_, String>("membership_state")?)?,
        exit_status,
    })
}

pub fn event_from_row(row: &Row<'_>) -> Result<DomainEvent, SqlError> {
    let envelope = EventEnvelope {
        event_id: model_core::ids::EventId::new(row.get("event_id")?),
        trace_id: model_core::ids::TraceId::new(row.get("trace_id")?),
        observed_at: decode_time(row.get("observed_at")?),
        process: ProcessIdentity {
            pid: row.get("process_pid")?,
            task_id: row.get("process_task_id")?,
            start_time_ticks: row.get("process_start_ticks")?,
            pid_namespace: row
                .get::<_, Option<String>>("process_pid_namespace")?
                .map(NamespaceIdentity::new),
            generation: row.get("process_generation")?,
        },
        collector: model_core::ids::CollectorName::new(row.get::<_, String>("collector")?),
        kind: decode_event_kind(&row.get::<_, String>("kind")?)?,
        flags: EventFlags {
            bootstrap_observed: i64_to_bool(row.get("bootstrap_observed")?),
            metadata_partial: i64_to_bool(row.get("metadata_partial")?),
            policy_modified: i64_to_bool(row.get("policy_modified")?),
        },
    };
    let payload = decode_event_payload(
        &row.get::<_, String>("payload_variant")?,
        &row.get::<_, String>("payload_fields")?,
        &row.get::<_, String>("payload_bytes")?,
    )?;
    let policy = decode_policy_record(
        &row.get::<_, String>("policy_verdict")?,
        row.get("policy_note")?,
        &row.get::<_, String>("policy_redactions")?,
        &row.get::<_, String>("policy_truncations")?,
    )?;
    Ok(DomainEvent {
        envelope,
        payload,
        policy,
    })
}

pub fn payload_segment_from_row(row: &Row<'_>) -> Result<PayloadSegment, SqlError> {
    Ok(PayloadSegment {
        segment_id: PayloadSegmentId::new(row.get("segment_id")?),
        trace_id: model_core::ids::TraceId::new(row.get("trace_id")?),
        observed_at: decode_time(row.get("observed_at")?),
        process: ProcessIdentity {
            pid: row.get("process_pid")?,
            task_id: row.get("process_task_id")?,
            start_time_ticks: row.get("process_start_ticks")?,
            pid_namespace: row
                .get::<_, Option<String>>("process_pid_namespace")?
                .map(NamespaceIdentity::new),
            generation: row.get("process_generation")?,
        },
        source_boundary: decode_payload_source_boundary(&row.get::<_, String>("source_boundary")?)?,
        content_state: decode_payload_content_state(&row.get::<_, String>("content_state")?)?,
        direction: decode_payload_direction(&row.get::<_, String>("direction")?)?,
        stream_key: PayloadStreamKey::new(row.get::<_, String>("stream_key")?),
        sequence: row.get("sequence")?,
        original_size: row.get("original_size")?,
        captured_size: row.get("captured_size")?,
        operation_id: row.get("operation_id")?,
        operation_offset: row.get("operation_offset")?,
        operation_original_size: row.get("operation_original_size")?,
        operation_captured_size: row.get("operation_captured_size")?,
        operation_completion_state: decode_payload_operation_completion_state(
            &row.get::<_, String>("operation_completion_state")?,
        )?,
        truncation: decode_payload_truncation_state(&row.get::<_, String>("truncation_state")?)?,
        redaction: decode_payload_redaction_state(&row.get::<_, String>("redaction_state")?)?,
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
            .get::<_, Option<u32>>("process_pid")?
            .map(|pid| ProcessIdentity {
                pid,
                task_id: row.get("process_task_id").ok().flatten(),
                start_time_ticks: row.get("process_start_ticks").unwrap_or_default(),
                pid_namespace: row
                    .get::<_, Option<String>>("process_pid_namespace")
                    .ok()
                    .flatten()
                    .map(NamespaceIdentity::new),
                generation: row.get("process_generation").unwrap_or_default(),
            }),
        kind: decode_diagnostic_kind(&row.get::<_, String>("kind")?)?,
        severity: decode_diagnostic_severity(&row.get::<_, String>("severity")?)?,
        emitted_at: decode_time(row.get("emitted_at")?),
        message: row.get("message")?,
        metadata: decode_map(&row.get::<_, String>("metadata")?),
    })
}

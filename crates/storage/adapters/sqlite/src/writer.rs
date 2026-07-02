//! Single-writer boundary for serialized SQLite writes.

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
use rusqlite::params;
use store_write_contract::WriteError;
use store_write_contract::diagnostics::DiagnosticWriteStore;
use store_write_contract::events::EventWriteStore;
use store_write_contract::memberships::MembershipWriteStore;
use store_write_contract::payloads::PayloadWriteStore;
use store_write_contract::traces::TraceWriteStore;

use crate::SqliteStorage;
use crate::records::{
    bool_to_i64, encode_diagnostic_kind, encode_diagnostic_severity, encode_event_kind,
    encode_event_payload, encode_exit_observation_source, encode_map, encode_membership_state,
    encode_payload_content_state, encode_payload_direction,
    encode_payload_operation_completion_state, encode_payload_redaction_state,
    encode_payload_source_boundary, encode_payload_truncation_state, encode_policy_record,
    encode_policy_verdict, encode_tags, encode_time, encode_trace_health, encode_trace_lifecycle,
};

impl TraceWriteStore for SqliteStorage {
    fn create_trace(&mut self, trace: TraceRecord) -> Result<(), WriteError> {
        self.connection()
            .borrow_mut()
            .execute(
                "INSERT OR REPLACE INTO traces (
                    trace_id, root_pid, root_task_id, root_start_ticks, root_pid_namespace,
                    root_container_id, root_generation, display_name, profile_name, tags,
                    lifecycle_state, health, created_at, started_at, completed_at, exited_at, failed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    trace.trace_id.get(),
                    trace.root_process_identity.pid,
                    trace.root_process_identity.task_id,
                    trace.root_process_identity.start_time_ticks,
                    trace
                        .root_process_identity
                        .pid_namespace
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                    trace.root_container_id.clone(),
                    trace.root_process_identity.generation,
                    trace.display_name.to_string(),
                    trace.profile_name.to_string(),
                    encode_tags(&trace.tags),
                    encode_trace_lifecycle(trace.lifecycle_state),
                    encode_trace_health(trace.health),
                    encode_time(trace.timings.created_at),
                    trace.timings.started_at.map(encode_time),
                    trace.timings.completed_at.map(encode_time),
                    trace.timings.exited_at.map(encode_time),
                    trace.timings.failed_at.map(encode_time),
                ],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("create_trace", error.to_string()))
    }

    fn update_trace_lifecycle(
        &mut self,
        trace_id: model_core::ids::TraceId,
        lifecycle_state: TraceLifecycleState,
    ) -> Result<(), WriteError> {
        self.connection()
            .borrow_mut()
            .execute(
                "UPDATE traces SET lifecycle_state = ?2 WHERE trace_id = ?1",
                params![trace_id.get(), encode_trace_lifecycle(lifecycle_state)],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("update_trace_lifecycle", error.to_string()))
    }

    fn update_trace_health(
        &mut self,
        trace_id: model_core::ids::TraceId,
        health: TraceHealth,
    ) -> Result<(), WriteError> {
        self.connection()
            .borrow_mut()
            .execute(
                "UPDATE traces SET health = ?2 WHERE trace_id = ?1",
                params![trace_id.get(), encode_trace_health(health)],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("update_trace_health", error.to_string()))
    }
}

impl MembershipWriteStore for SqliteStorage {
    fn upsert_membership(&mut self, membership: ProcessMembership) -> Result<(), WriteError> {
        self.connection()
            .borrow_mut()
            .execute(
                "INSERT OR REPLACE INTO memberships (
                    trace_id, pid, task_id, start_ticks, pid_namespace, generation,
                    inherited_from_pid, inherited_from_task_id, inherited_from_start_ticks,
                    inherited_from_pid_namespace, inherited_from_generation, observed_at,
                    capture_enabled, propagation_enabled, membership_state, exit_code,
                    exit_observed_at, exit_observation_source
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    membership.trace_id.get(),
                    membership.identity.pid,
                    membership.identity.task_id,
                    membership.identity.start_time_ticks,
                    membership
                        .identity
                        .pid_namespace
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                    membership.identity.generation,
                    membership
                        .inherited_from
                        .as_ref()
                        .map(|identity| identity.pid),
                    membership
                        .inherited_from
                        .as_ref()
                        .and_then(|identity| identity.task_id),
                    membership
                        .inherited_from
                        .as_ref()
                        .map(|identity| identity.start_time_ticks),
                    membership.inherited_from.as_ref().and_then(|identity| {
                        identity
                            .pid_namespace
                            .as_ref()
                            .map(|value| value.as_str().to_string())
                    }),
                    membership
                        .inherited_from
                        .as_ref()
                        .map(|identity| identity.generation),
                    membership.observed_at.map(encode_time),
                    bool_to_i64(membership.capture_enabled),
                    bool_to_i64(membership.propagation_enabled),
                    encode_membership_state(membership.state),
                    membership.exit_status.as_ref().and_then(|value| value.code),
                    membership
                        .exit_status
                        .as_ref()
                        .map(|value| encode_time(value.observed_at)),
                    membership
                        .exit_status
                        .as_ref()
                        .and_then(|value| value.source)
                        .map(encode_exit_observation_source),
                ],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("upsert_membership", error.to_string()))
    }
}

impl EventWriteStore for SqliteStorage {
    fn append_event(&mut self, event: DomainEvent) -> Result<(), WriteError> {
        let (payload_variant, payload_fields, payload_bytes) = encode_event_payload(&event.payload);
        let (policy_redactions, policy_truncations) = encode_policy_record(&event.policy);
        self.connection()
            .borrow_mut()
            .execute(
                "INSERT OR REPLACE INTO events (
                    event_id, trace_id, observed_at, process_pid, process_task_id, process_start_ticks,
                    process_pid_namespace, process_generation, collector, kind, bootstrap_observed,
                    metadata_partial, policy_modified, payload_variant, payload_fields, payload_bytes,
                    policy_verdict, policy_note, policy_redactions, policy_truncations
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![
                    event.envelope.event_id.get(),
                    event.envelope.trace_id.get(),
                    encode_time(event.envelope.observed_at),
                    event.envelope.process.pid,
                    event.envelope.process.task_id,
                    event.envelope.process.start_time_ticks,
                    event.envelope
                        .process
                        .pid_namespace
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                    event.envelope.process.generation,
                    event.envelope.collector.to_string(),
                    encode_event_kind(event.envelope.kind),
                    bool_to_i64(event.envelope.flags.bootstrap_observed),
                    bool_to_i64(event.envelope.flags.metadata_partial),
                    bool_to_i64(event.envelope.flags.policy_modified),
                    payload_variant,
                    payload_fields,
                    payload_bytes,
                    encode_policy_verdict(event.policy.verdict),
                    event.policy.note,
                    policy_redactions,
                    policy_truncations,
                ],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("append_event", error.to_string()))
    }
}

impl PayloadWriteStore for SqliteStorage {
    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), WriteError> {
        self.connection()
            .borrow_mut()
            .execute(
                "INSERT OR REPLACE INTO payload_segments (
                    segment_id, trace_id, observed_at, process_pid, process_task_id,
                    process_start_ticks, process_pid_namespace, process_generation,
                    source_boundary, content_state, direction, stream_key, sequence,
                    original_size, captured_size, operation_id, operation_offset,
                    operation_original_size, operation_captured_size, operation_completion_state,
                    truncation_state, redaction_state, library, symbol, protocol_hint, bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                params![
                    segment.segment_id.get(),
                    segment.trace_id.get(),
                    encode_time(segment.observed_at),
                    segment.process.pid,
                    segment.process.task_id,
                    segment.process.start_time_ticks,
                    segment
                        .process
                        .pid_namespace
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                    segment.process.generation,
                    encode_payload_source_boundary(segment.source_boundary),
                    encode_payload_content_state(segment.content_state),
                    encode_payload_direction(segment.direction),
                    segment.stream_key.to_string(),
                    segment.sequence,
                    segment.original_size,
                    segment.captured_size,
                    segment.operation_id,
                    segment.operation_offset,
                    segment.operation_original_size,
                    segment.operation_captured_size,
                    encode_payload_operation_completion_state(segment.operation_completion_state),
                    encode_payload_truncation_state(segment.truncation),
                    encode_payload_redaction_state(segment.redaction),
                    segment.library,
                    segment.symbol,
                    segment.protocol_hint,
                    segment.bytes,
                ],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("append_payload_segment", error.to_string()))
    }
}

impl DiagnosticWriteStore for SqliteStorage {
    fn append_diagnostic(&mut self, diagnostic: DiagnosticRecord) -> Result<(), WriteError> {
        self.connection()
            .borrow_mut()
            .execute(
                "INSERT OR REPLACE INTO diagnostics (
                    diagnostic_id, trace_id, process_pid, process_task_id, process_start_ticks,
                    process_pid_namespace, process_generation, kind, severity, emitted_at, message, metadata
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    diagnostic.diagnostic_id.get(),
                    diagnostic.trace_id.map(|value| value.get()),
                    diagnostic.process.as_ref().map(|value| value.pid),
                    diagnostic.process.as_ref().and_then(|value| value.task_id),
                    diagnostic.process.as_ref().map(|value| value.start_time_ticks),
                    diagnostic.process.as_ref().and_then(|value| {
                        value
                            .pid_namespace
                            .as_ref()
                            .map(|namespace| namespace.as_str().to_string())
                    }),
                    diagnostic.process.as_ref().map(|value| value.generation),
                    encode_diagnostic_kind(diagnostic.kind),
                    encode_diagnostic_severity(diagnostic.severity),
                    encode_time(diagnostic.emitted_at),
                    diagnostic.message,
                    encode_map(&diagnostic.metadata),
                ],
            )
            .map(|_| ())
            .map_err(|error| WriteError::new("append_diagnostic", error.to_string()))
    }
}

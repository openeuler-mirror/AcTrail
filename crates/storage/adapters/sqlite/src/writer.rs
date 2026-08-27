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
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT INTO traces (
                    trace_id, otel_trace_id, alert_token, root_process_id, root_container_id, root_working_directory,
                    display_name, profile_name, tags, lifecycle_state, health, created_at,
                    started_at, completed_at, exited_at, failed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT(trace_id) DO UPDATE SET
                    otel_trace_id = excluded.otel_trace_id,
                    alert_token = excluded.alert_token,
                    root_process_id = excluded.root_process_id,
                    root_container_id = excluded.root_container_id,
                    root_working_directory = excluded.root_working_directory,
                    display_name = excluded.display_name,
                    profile_name = excluded.profile_name,
                    tags = excluded.tags,
                    lifecycle_state = excluded.lifecycle_state,
                    health = excluded.health,
                    created_at = excluded.created_at,
                    started_at = excluded.started_at,
                    completed_at = excluded.completed_at,
                    exited_at = excluded.exited_at,
                    failed_at = excluded.failed_at",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    trace.trace_id.get(),
                    trace.otel_trace_id.as_bytes().as_slice(),
                    trace.alert_token.as_bytes().as_slice(),
                    trace.root_process_identity.get(),
                    trace.root_container_id.clone(),
                    trace.root_working_directory.clone(),
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
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("create_trace", error.to_string()))
    }

    fn update_trace_lifecycle(
        &mut self,
        trace_id: model_core::ids::TraceId,
        lifecycle_state: TraceLifecycleState,
    ) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached("UPDATE traces SET lifecycle_state = ?2 WHERE trace_id = ?1")
            .and_then(|mut statement| {
                statement.execute(params![
                    trace_id.get(),
                    encode_trace_lifecycle(lifecycle_state)
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("update_trace_lifecycle", error.to_string()))
    }

    fn update_trace_health(
        &mut self,
        trace_id: model_core::ids::TraceId,
        health: TraceHealth,
    ) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached("UPDATE traces SET health = ?2 WHERE trace_id = ?1")
            .and_then(|mut statement| {
                statement.execute(params![trace_id.get(), encode_trace_health(health)])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("update_trace_health", error.to_string()))
    }
}

impl MembershipWriteStore for SqliteStorage {
    fn upsert_membership(&mut self, membership: ProcessMembership) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT INTO memberships (
                    trace_id, process_id, inherited_from_process_id, observed_at,
                    capture_enabled, propagation_enabled, membership_state, exit_code,
                    exit_observed_at, exit_observation_source
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT(trace_id, process_id) DO UPDATE SET
                    inherited_from_process_id = excluded.inherited_from_process_id,
                    observed_at = excluded.observed_at,
                    capture_enabled = excluded.capture_enabled,
                    propagation_enabled = excluded.propagation_enabled,
                    membership_state = excluded.membership_state,
                    exit_code = excluded.exit_code,
                    exit_observed_at = excluded.exit_observed_at,
                    exit_observation_source = excluded.exit_observation_source
                WHERE memberships.inherited_from_process_id IS NOT excluded.inherited_from_process_id
                   OR memberships.observed_at IS NOT excluded.observed_at
                   OR memberships.capture_enabled IS NOT excluded.capture_enabled
                   OR memberships.propagation_enabled IS NOT excluded.propagation_enabled
                   OR memberships.membership_state IS NOT excluded.membership_state
                   OR memberships.exit_code IS NOT excluded.exit_code
                   OR memberships.exit_observed_at IS NOT excluded.exit_observed_at
                   OR memberships.exit_observation_source IS NOT excluded.exit_observation_source",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    membership.trace_id.get(),
                    membership.identity.get(),
                    membership.inherited_from.map(|identity| identity.get()),
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
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("upsert_membership", error.to_string()))
    }
}

impl EventWriteStore for SqliteStorage {
    fn append_event(&mut self, mut event: DomainEvent) -> Result<(), WriteError> {
        let encoded = encode_event_payload(&mut event.payload)
            .map_err(|error| WriteError::new("encode_event_payload", error.to_string()))?;
        let (policy_redactions, policy_truncations) = encode_policy_record(&event.policy);
        let connection = self.connection().borrow_mut();
        let mut block_ids = Vec::with_capacity(encoded.blocks.len());
        for block in &encoded.blocks {
            let compressed = zstd::stream::encode_all(
                block.bytes.as_slice(),
                self.cold_field_compression.zstd_level,
            )
            .map_err(|error| WriteError::new("encode_event_payload_block", error.to_string()))?;
            connection
                .execute(
                    "INSERT INTO event_payload_blocks (trace_id, kind, encoded_bytes)
                     VALUES (?1, ?2, ?3)",
                    params![
                        event.envelope.trace_id.get(),
                        block.kind.to_i64(),
                        compressed
                    ],
                )
                .map_err(|error| {
                    WriteError::new("insert_event_payload_block", error.to_string())
                })?;
            block_ids.push(connection.last_insert_rowid());
        }
        let payload_blocks = block_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO events (
                    event_id, trace_id, observed_at, process_id, collector, kind, bootstrap_observed,
                    metadata_partial, policy_modified, payload_variant, payload, payload_code,
                    payload_blocks, policy_verdict, policy_note, policy_redactions, policy_truncations
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    event.envelope.event_id.get(),
                    event.envelope.trace_id.get(),
                    encode_time(event.envelope.observed_at),
                    event.envelope.process.get(),
                    event.envelope.collector.to_string(),
                    encode_event_kind(event.envelope.kind),
                    bool_to_i64(event.envelope.flags.bootstrap_observed),
                    bool_to_i64(event.envelope.flags.metadata_partial),
                    bool_to_i64(event.envelope.flags.policy_modified),
                    encoded.variant,
                    encoded.fields,
                    1i64,
                    payload_blocks,
                    encode_policy_verdict(event.policy.verdict),
                    event.policy.note,
                    policy_redactions,
                    policy_truncations,
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("append_event", error.to_string()))
    }
}

impl PayloadWriteStore for SqliteStorage {
    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO payload_segments (
                    segment_id, trace_id, observed_at, process_id, source_boundary,
                    content_state, direction, stream_key, sequence,
                    original_size, captured_size, operation_id, operation_offset,
                    operation_original_size, operation_captured_size, operation_completion_state,
                    truncation_state, redaction_state, library, symbol, protocol_hint, bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    segment.segment_id.get(),
                    segment.trace_id.get(),
                    encode_time(segment.observed_at),
                    segment.process.get(),
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
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("append_payload_segment", error.to_string()))
    }
}

impl DiagnosticWriteStore for SqliteStorage {
    fn append_diagnostic(&mut self, diagnostic: DiagnosticRecord) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO diagnostics (
                    diagnostic_id, trace_id, process_id, kind, severity, emitted_at, message, metadata
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    diagnostic.diagnostic_id.get(),
                    diagnostic.trace_id.map(|value| value.get()),
                    diagnostic.process.map(|value| value.get()),
                    encode_diagnostic_kind(diagnostic.kind),
                    encode_diagnostic_severity(diagnostic.severity),
                    encode_time(diagnostic.emitted_at),
                    diagnostic.message,
                    encode_map(&diagnostic.metadata),
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("append_diagnostic", error.to_string()))
    }
}

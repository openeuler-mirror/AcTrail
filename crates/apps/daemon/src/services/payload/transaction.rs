//! Payload segment policy and persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use config_core::daemon::{DiagnosticLogLevel, PayloadStdioStorageMode, SemanticRetentionConfig};
use control_contract::reply::ControlError;
use model_core::capability::{Capability, RequestMode};
use model_core::event::{DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload};
use model_core::ids::{CollectorName, EventId, TraceId};
use model_core::payload::{PayloadRedactionState, PayloadSegment, PayloadSegmentId};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use recording_runtime::{
    ObservedRecordWriteSession, RecordingError, RecordingWriter, SemanticActionBatch,
};
use semantic_action_runtime::LiveSemanticActionRuntime;
use trace_runtime::registry::TraceRuntime;

use crate::services::application_protocol::{
    ApplicationEventDraft, ApplicationProtocolAnalyzer,
    COLLECTOR_NAME as APPLICATION_PROTOCOL_COLLECTOR_NAME,
};
use crate::services::attach::StorageAttachService;
use crate::services::diagnostic_logging;
use crate::services::identity::TraceIdentityResolver;
use crate::services::live::next_diagnostic_id_from_seed;
use crate::services::payload_gate::{
    PayloadBodyRetention, PayloadBodyRetentionDecision, PayloadBodyRetentionGate,
    SocketHttpPayloadGate,
};
use crate::services::semantic_actions::LiveTraceRecordLookup;
use crate::services::workload_diagnostics::{PayloadTransactionPhase, WorkloadDiagnostics};

use super::policy::{
    PayloadPolicyConfig, apply_stdio_storage_mode, should_clear_transport_payload_body,
};
use super::redaction::redact_payload_bytes;
use super::retention::RetainedPayloadTransaction;

impl StorageAttachService {
    #[cfg(test)]
    pub(in crate::services) fn process_payload_segment_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw: RawPayloadSegment,
    ) -> Result<(), ControlError> {
        self.process_payload_segments_impl(trace_runtime, vec![raw])
    }

    pub(in crate::services) fn process_payload_segments_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw_segments: Vec<RawPayloadSegment>,
    ) -> Result<(), ControlError> {
        if raw_segments.is_empty() {
            return Ok(());
        }
        let raw_segment_count = raw_segments.len();
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        let mut semantic_action_count = 0usize;
        let mut semantic_link_count = 0usize;
        let mut retained_payload_transaction = RetainedPayloadTransaction::default();
        let started = crate::services::workload_diagnostics::now();
        let result = {
            let mut context = PayloadTransactionContext {
                trace_runtime,
                socket_payload_gate: &mut self.socket_payload_gate,
                payload_body_retention_gate: &mut self.payload_body_retention_gate,
                semantic_retention: &self.semantic_retention,
                application_protocol: &mut self.application_protocol,
                semantic_actions: &mut self.semantic_actions,
                finalized_terminal_traces: &mut self.finalized_terminal_traces,
                workload_diagnostics: &self.workload_diagnostics,
                retained_payload_bytes_by_trace: &mut self.retained_payload_bytes_by_trace,
                retained_payload_transaction: &mut retained_payload_transaction,
                next_event_id: &mut self.next_event_id,
                next_payload_segment_id: &mut self.next_payload_segment_id,
                diagnostic_log_level: self.diagnostic_log_level,
                policy: PayloadPolicyConfig {
                    tls_enabled: self.payload_tls_enabled,
                    tls_redaction_policy: self.payload_tls_redaction_policy,
                    tls_retention_max_bytes_per_trace: self
                        .payload_tls_retention_max_bytes_per_trace,
                    stdio_enabled: self.payload_stdio_enabled,
                    stdio_redaction_policy: self.payload_stdio_redaction_policy,
                    stdio_retention_max_bytes_per_trace: self
                        .payload_stdio_retention_max_bytes_per_trace,
                    stdio_stdin_storage_mode: self.payload_stdio_stdin_storage_mode,
                    stdio_stdout_storage_mode: self.payload_stdio_stdout_storage_mode,
                    stdio_stderr_storage_mode: self.payload_stdio_stderr_storage_mode,
                    socket_enabled: self.payload_socket_enabled,
                    socket_redaction_policy: self.payload_socket_redaction_policy,
                    socket_retention_max_bytes_per_trace: self
                        .payload_socket_retention_max_bytes_per_trace,
                },
            };
            RecordingWriter::new(self.storage.as_mut())
                .write_session_then_export(
                    &self.export_runtime,
                    &traces,
                    SystemTime::now(),
                    || {
                        next_diagnostic_id_from_seed(next_diagnostic_id)
                            .map_err(control_error_to_recording)
                    },
                    |session| {
                        let semantic_actions = context
                            .process_payload_segments(session, raw_segments)
                            .map_err(control_error_to_recording)?;
                        semantic_action_count = semantic_actions.actions().len();
                        semantic_link_count = semantic_actions.links().len();
                        Ok(semantic_actions)
                    },
                )
                .map_err(recording_error_to_control)
        };
        retained_payload_transaction
            .apply_result(&mut self.retained_payload_bytes_by_trace, &result);
        self.workload_diagnostics.record_storage_batch(
            started.elapsed(),
            0,
            raw_segment_count,
            0,
            semantic_action_count,
            semantic_link_count,
            0,
            result.is_ok(),
        );
        result
    }
}

struct PayloadTransactionContext<'a> {
    trace_runtime: &'a TraceRuntime,
    socket_payload_gate: &'a mut SocketHttpPayloadGate,
    payload_body_retention_gate: &'a mut PayloadBodyRetentionGate,
    semantic_retention: &'a SemanticRetentionConfig,
    application_protocol: &'a mut ApplicationProtocolAnalyzer,
    semantic_actions: &'a mut LiveSemanticActionRuntime,
    finalized_terminal_traces: &'a mut BTreeSet<TraceId>,
    workload_diagnostics: &'a WorkloadDiagnostics,
    retained_payload_bytes_by_trace: &'a mut BTreeMap<TraceId, u64>,
    retained_payload_transaction: &'a mut RetainedPayloadTransaction,
    next_event_id: &'a mut u64,
    next_payload_segment_id: &'a mut u64,
    diagnostic_log_level: DiagnosticLogLevel,
    policy: PayloadPolicyConfig,
}

impl PayloadTransactionContext<'_> {
    fn process_payload_segments(
        &mut self,
        session: &mut ObservedRecordWriteSession<'_>,
        raw_segments: Vec<RawPayloadSegment>,
    ) -> Result<SemanticActionBatch, ControlError> {
        let mut semantic_actions = SemanticActionBatch::default();
        for raw in raw_segments {
            semantic_actions.extend(self.process_admitted_payload_segment(session, raw)?);
        }
        Ok(semantic_actions)
    }

    fn process_admitted_payload_segment(
        &mut self,
        session: &mut ObservedRecordWriteSession<'_>,
        raw: RawPayloadSegment,
    ) -> Result<SemanticActionBatch, ControlError> {
        let admitted = self
            .socket_payload_gate
            .admit(raw)
            .map_err(|error| ControlError::new("socket_payload_gate", error))?;
        let mut semantic_actions = SemanticActionBatch::default();
        for raw in admitted {
            semantic_actions.extend(self.persist_payload_segment(session, raw)?);
        }
        Ok(semantic_actions)
    }

    fn persist_payload_segment(
        &mut self,
        session: &mut ObservedRecordWriteSession<'_>,
        raw: RawPayloadSegment,
    ) -> Result<SemanticActionBatch, ControlError> {
        let Some(matched) = TraceIdentityResolver::new(self.trace_runtime).payload_process(&raw)
        else {
            self.log_payload_diagnostic(format_args!(
                "payload_persist drop_membership_miss trace_id={} pid={} generation={} source={:?} operation_id={}",
                raw.trace_id,
                raw.process.pid,
                raw.process.generation,
                raw.source_boundary,
                raw.operation_id
            ));
            return Ok(SemanticActionBatch::default());
        };
        if matched.trace_id != raw.trace_id {
            self.log_payload_diagnostic(format_args!(
                "payload_persist drop_trace_mismatch raw_trace_id={} matched_trace_id={} pid={} generation={} source={:?} operation_id={}",
                raw.trace_id,
                matched.trace_id,
                raw.process.pid,
                raw.process.generation,
                raw.source_boundary,
                raw.operation_id
            ));
            return Ok(SemanticActionBatch::default());
        }
        let policy = self.policy.for_segment(&raw)?;
        if matches!(policy.stdio_storage_mode, PayloadStdioStorageMode::Drop) {
            self.log_payload_diagnostic(format_args!(
                "payload_persist drop_stdio_storage_policy trace_id={} pid={} generation={} stream={} operation_id={}",
                raw.trace_id,
                raw.process.pid,
                raw.process.generation,
                raw.protocol_hint.as_deref().unwrap_or("unknown"),
                raw.operation_id
            ));
            return Ok(SemanticActionBatch::default());
        }

        self.mark_semantic_projection_dirty(raw.trace_id);

        let mut segment = PayloadSegment {
            segment_id: self.next_payload_segment_id()?,
            trace_id: raw.trace_id,
            observed_at: raw.observed_at,
            process: matched.process.clone(),
            source_boundary: raw.source_boundary,
            content_state: raw.content_state,
            direction: raw.direction,
            stream_key: raw.stream_key,
            sequence: raw.sequence,
            original_size: raw.original_size,
            captured_size: raw.captured_size,
            operation_id: raw.operation_id,
            operation_offset: raw.operation_offset,
            operation_original_size: raw.operation_original_size,
            operation_captured_size: raw.operation_captured_size,
            operation_completion_state: raw.operation_completion_state,
            truncation: raw.truncation,
            redaction: PayloadRedactionState::NotRequired,
            library: raw.library,
            symbol: raw.symbol,
            protocol_hint: raw.protocol_hint,
            bytes: raw.bytes,
        };
        let body_retention: PayloadBodyRetentionDecision =
            self.payload_body_retention_gate.decide(&segment);
        let (bytes, redaction) =
            redact_payload_bytes(policy.redaction, std::mem::take(&mut segment.bytes));
        segment.captured_size = bytes.len() as u64;
        segment.redaction = redaction;
        segment.bytes = bytes;
        let analysis_segment = segment.clone();
        let mut stored_segment = segment;
        if should_clear_transport_payload_body(
            &stored_segment,
            self.semantic_retention,
            body_retention.semantic_layer.consumed_by_higher_layer(),
        ) {
            stored_segment.bytes.clear();
        }
        apply_stdio_storage_mode(&mut stored_segment, policy.stdio_storage_mode);
        let retained_body_bytes = u64::try_from(stored_segment.bytes.len())
            .map_err(|error| ControlError::new("payload_retention", error.to_string()))?;
        let started = crate::services::workload_diagnostics::now();
        let retained_bytes = self.retained_payload_bytes(session, raw.trace_id)?;
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::RetentionCheck,
            started.elapsed(),
            0,
        );
        let next_retained_bytes = retained_bytes
            .checked_add(retained_body_bytes)
            .ok_or_else(|| ControlError::new("payload_retention", "payload retention overflow"))?;
        if next_retained_bytes > policy.retention_max_bytes_per_trace {
            return Err(ControlError::new(
                "payload_retention",
                format!(
                    "trace {} payload retention would exceed configured maximum {} bytes",
                    raw.trace_id, policy.retention_max_bytes_per_trace
                ),
            ));
        }
        let stored_captured_size = stored_segment.captured_size;
        let started = crate::services::workload_diagnostics::now();
        let semantic_actions = self.observe_semantic_actions_for_payload_segment(&analysis_segment);
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::SemanticObserve,
            started.elapsed(),
            semantic_actions.actions().len(),
        );
        let started = crate::services::workload_diagnostics::now();
        let mut semantic_actions = session
            .persist_payload_segment(stored_segment, semantic_actions)
            .map_err(recording_error_to_control)?;
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::SegmentPersist,
            started.elapsed(),
            semantic_actions.actions().len(),
        );
        self.retained_payload_transaction
            .record_persisted(raw.trace_id, next_retained_bytes);
        self.log_payload_diagnostic(format_args!(
            "payload_persist stored trace_id={} pid={} generation={} source={:?} captured_bytes={} retained_body_bytes={} operation_id={}",
            raw.trace_id,
            matched.process.pid,
            matched.process.generation,
            raw.source_boundary,
            stored_captured_size,
            retained_body_bytes,
            raw.operation_id
        ));
        let started = crate::services::workload_diagnostics::now();
        let application_drafts =
            if application_protocol_requested(self.trace_runtime, raw.trace_id)? {
                self.application_protocol
                    .analyze_with_semantic_context(
                        &analysis_segment,
                        body_retention.semantic_layer.consumed_by_llm(),
                        matches!(body_retention.mode, PayloadBodyRetention::SummaryOnly),
                    )
                    .map_err(|error| ControlError::new("application_protocol_analyzer", error))?
            } else {
                Vec::new()
            };
        let application_draft_count = application_drafts.len();
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::ApplicationAnalyze,
            started.elapsed(),
            application_draft_count,
        );
        let started = crate::services::workload_diagnostics::now();
        let application_actions = self.persist_application_events(
            session,
            raw.trace_id,
            raw.observed_at,
            matched.process,
            application_drafts,
        )?;
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::ApplicationPersist,
            started.elapsed(),
            application_draft_count,
        );
        semantic_actions.extend(application_actions);
        self.payload_body_retention_gate
            .apply(&analysis_segment, body_retention);
        Ok(semantic_actions)
    }

    fn next_payload_segment_id(&mut self) -> Result<PayloadSegmentId, ControlError> {
        let raw = *self.next_payload_segment_id;
        *self.next_payload_segment_id =
            (*self.next_payload_segment_id)
                .checked_add(1)
                .ok_or_else(|| {
                    ControlError::new("payload_segment_id_overflow", "payload segment id overflow")
                })?;
        Ok(PayloadSegmentId::new(raw))
    }

    fn retained_payload_bytes(
        &mut self,
        session: &ObservedRecordWriteSession<'_>,
        trace_id: TraceId,
    ) -> Result<u64, ControlError> {
        self.retained_payload_transaction.bytes(
            self.retained_payload_bytes_by_trace,
            session,
            trace_id,
        )
    }

    fn persist_application_events(
        &mut self,
        session: &mut ObservedRecordWriteSession<'_>,
        trace_id: TraceId,
        observed_at: SystemTime,
        process: ProcessIdentity,
        drafts: Vec<ApplicationEventDraft>,
    ) -> Result<SemanticActionBatch, ControlError> {
        let mut semantic_actions = SemanticActionBatch::default();
        for draft in drafts {
            let event = DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id,
                    observed_at,
                    process: process.clone(),
                    collector: CollectorName::new(APPLICATION_PROTOCOL_COLLECTOR_NAME),
                    kind: EventKind::Application,
                    flags: EventFlags::clean(),
                },
                EventPayload::Application(draft.payload),
            );
            let event_actions = self.observe_semantic_actions_for_event(&event);
            let event_actions = session
                .persist_event(event, event_actions)
                .map_err(recording_error_to_control)?;
            semantic_actions.extend(event_actions);
        }
        Ok(semantic_actions)
    }

    fn log_payload_diagnostic(&self, args: std::fmt::Arguments<'_>) {
        diagnostic_logging::log_diagnostic(
            self.diagnostic_log_level,
            DiagnosticLogLevel::Debug,
            args,
        );
    }

    fn next_event_id(&mut self) -> Result<EventId, ControlError> {
        let raw = *self.next_event_id;
        *self.next_event_id = (*self.next_event_id)
            .checked_add(1)
            .ok_or_else(|| ControlError::new("event_id_overflow", "event id overflow"))?;
        Ok(EventId::new(raw))
    }

    fn mark_semantic_projection_dirty(&mut self, trace_id: TraceId) {
        self.finalized_terminal_traces.remove(&trace_id);
    }

    fn observe_semantic_actions_for_event(&mut self, event: &DomainEvent) -> SemanticActionBatch {
        let output = self.semantic_actions.observe_event(event);
        SemanticActionBatch::from_action_output(
            output.actions,
            output.links,
            output.file_observation_paths,
            output.file_path_sets,
        )
    }

    fn observe_semantic_actions_for_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> SemanticActionBatch {
        let output = self.semantic_actions.observe_payload_segment(segment);
        SemanticActionBatch::from_action_output(
            output.actions,
            output.links,
            output.file_observation_paths,
            output.file_path_sets,
        )
    }
}

fn recording_error_to_control(error: RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn control_error_to_recording(error: ControlError) -> RecordingError {
    RecordingError::new(error.code, error.message)
}

fn application_protocol_requested(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<bool, ControlError> {
    let entry = trace_runtime
        .get_trace(trace_id)
        .ok_or_else(|| ControlError::new("payload_match", "payload trace does not exist"))?;
    Ok(entry
        .profile_snapshot
        .capability_requests
        .iter()
        .any(|request| {
            request.mode != RequestMode::Disabled
                && matches!(
                    request.capability,
                    Capability::NetApplicationPlaintextHttp | Capability::NetApplicationHttp2Frames
                )
        }))
}

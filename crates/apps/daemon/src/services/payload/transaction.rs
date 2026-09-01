//! Payload segment policy and persistence.

mod application;
mod semantic_persistence;

use application::{application_protocol_requested, tls_summary_application_draft};

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use config_core::daemon::{DiagnosticLogLevel, PayloadStdioStorageMode, SemanticRetentionConfig};
use control_contract::reply::ControlError;
use model_core::capability::{Capability, RequestMode};
use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::event::{
    ApplicationPayload, DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload,
};
use model_core::ids::{CollectorName, EventId, TraceId};
use model_core::payload::{
    PayloadRedactionState, PayloadSegment, PayloadSegmentId, PayloadStreamIdentity,
};
use model_core::process::{ProcessIdentity, ProcessMembership};
use payload_event::{RawPayloadSegment, RawPayloadStreamClose};
use recording_runtime::{
    ObservedRecordWriteSession, RecordingError, RecordingWriter, SemanticActionBatch,
};
use semantic_action_runtime::LiveSemanticActionRuntime;
use semantic_action_runtime::live::LiveMcpStdioDiagnostic;
use storage_core::StorageBackend;
use trace_runtime::registry::TraceRuntime;

use crate::services::application_protocol::{
    ApplicationEventDraft, ApplicationProtocolAnalyzer,
    COLLECTOR_NAME as APPLICATION_PROTOCOL_COLLECTOR_NAME, IncompleteHttp1Message,
};
use crate::services::attach::StorageAttachService;
use crate::services::diagnostic_logging;
use crate::services::live::next_diagnostic_id_from_seed;
use crate::services::payload_gate::{
    PayloadBodyRetention, PayloadBodyRetentionDecision, PayloadBodyRetentionGate,
};
use crate::services::semantic_actions::LiveTraceRecordLookup;
use crate::services::workload_diagnostics::{PayloadTransactionPhase, WorkloadDiagnostics};

use super::policy::{
    PayloadPolicyConfig, apply_stdio_storage_mode, should_clear_transport_payload_body,
};
use super::redaction::redact_payload_bytes;
use super::retention::RetainedPayloadTransaction;

impl StorageAttachService {
    pub(in crate::services) fn process_payload_stream_closes_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        closes: Vec<RawPayloadStreamClose>,
    ) -> Result<(), ControlError> {
        for close in closes {
            self.socket_payload_gate.forget_stream(&close);
            self.payload_reorderer.forget_stream(&close);
            let (process, _) = self.resolve_process_observation(close.process.clone())?;
            let identity = PayloadStreamIdentity {
                trace_id: close.trace_id,
                process,
                source_boundary: close.source_boundary,
                stream_key: close.stream_key,
            };
            self.application_protocol.forget_stream(&identity);
            self.payload_body_retention_gate.forget_stream(&identity);
            let mut output = self
                .semantic_actions
                .finalize_payload_stream(&identity, close.observed_at);
            self.persist_llm_pipeline_diagnostics_fail_local(
                trace_runtime,
                std::mem::take(&mut output.llm_pipeline_diagnostics),
            );
            let batch = SemanticActionBatch::from_action_output(
                output.actions,
                output.links,
                output.file_observation_paths,
                output.file_path_sets,
                output.llm_request_contents,
                output.llm_request_lineages,
                output.mcp_jsonrpc_contents,
                output.payload_segments,
            );
            if !batch.actions().is_empty() || !batch.links().is_empty() {
                let persisted = self.write_semantic_action_batch(batch)?;
                self.publish_live_export_actions(trace_runtime, identity.trace_id, persisted)?;
            }
        }
        Ok(())
    }

    pub(in crate::services) fn process_payload_segments_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        raw_segments: Vec<RawPayloadSegment>,
    ) -> Result<(), ControlError> {
        let mut admitted = Vec::new();
        for raw in raw_segments {
            admitted.extend(
                self.socket_payload_gate
                    .admit(raw)
                    .map_err(|error| ControlError::new("socket_payload_gate", error))?,
            );
        }
        // Reorder kernel capture across per-CPU drain cycles and seccomp TLS
        // across completion cycles. Synchronous TLS capture is already FIFO
        // and preserves its arrival order.
        let admitted = self.payload_reorderer.admit(SystemTime::now(), admitted);
        if admitted.is_empty() {
            return Ok(());
        }
        let raw_segment_count = admitted.len();
        let mut resolved_segments = Vec::with_capacity(admitted.len());
        let mut process_records = BTreeMap::new();
        let mut membership_trace_ids = BTreeSet::new();
        for admitted in admitted {
            let raw = admitted.segment;
            let (process, record) = self.resolve_process_observation(raw.process.clone())?;
            if let Some(record) = record {
                process_records.insert(record.identity, record);
            }
            if trace_runtime
                .find_membership_in_trace(raw.trace_id, &process)
                .is_none()
            {
                trace_runtime
                    .insert_membership(
                        raw.trace_id,
                        ProcessMembership::observed(raw.trace_id, process, raw.observed_at),
                    )
                    .map_err(|error| {
                        ControlError::new("payload_membership", format!("{error:?}"))
                    })?;
                membership_trace_ids.insert(raw.trace_id);
            }
            resolved_segments.push(ResolvedRawPayloadSegment {
                raw,
                process,
                discontinuity_before: admitted.discontinuity_before,
            });
        }
        let membership_trace_states = membership_trace_ids
            .into_iter()
            .map(|trace_id| self.trace_state_record_for_persistence(trace_runtime, trace_id))
            .collect::<Result<Vec<_>, _>>()?;
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        let semantic_action_count;
        let semantic_link_count;
        let mut mcp_stdio_diagnostics = Vec::new();
        let mut llm_pipeline_diagnostics = Vec::new();
        let mut retained_payload_transaction = RetainedPayloadTransaction::default();
        let semantic_flush_diagnostics = self.workload_diagnostics.clone();
        let started = crate::services::workload_diagnostics::now();
        let result = {
            let mut context = PayloadTransactionContext {
                trace_runtime,
                payload_body_retention_gate: &mut self.payload_body_retention_gate,
                semantic_retention: &self.semantic_retention,
                application_protocol: &mut self.application_protocol,
                semantic_actions: &mut self.semantic_actions,
                mcp_stdio_diagnostics: &mut mcp_stdio_diagnostics,
                llm_pipeline_diagnostics: &mut llm_pipeline_diagnostics,
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
            let mut export_batch = SemanticActionBatch::default();
            let prepared = context.prepare_payload_segments(
                self.storage.as_mut(),
                resolved_segments,
                &mut export_batch,
            );
            semantic_action_count = export_batch.actions().len();
            semantic_link_count = export_batch.links().len();
            let result = RecordingWriter::new(self.storage.as_mut())
                .write_session_then_export(
                    &self.export_runtime,
                    &traces,
                    SystemTime::now(),
                    || {
                        next_diagnostic_id_from_seed(next_diagnostic_id)
                            .map_err(control_error_to_recording)
                    },
                    export_batch,
                    move |elapsed| {
                        semantic_flush_diagnostics.record_payload_transaction_phase(
                            PayloadTransactionPhase::SemanticPersist,
                            elapsed,
                            semantic_action_count + semantic_link_count,
                        );
                    },
                    |session| {
                        let prepared = prepared.map_err(control_error_to_recording)?;
                        for record in process_records.into_values() {
                            session.persist_process_record(record)?;
                        }
                        for trace_state in membership_trace_states {
                            session.persist_trace_state(trace_state)?;
                        }
                        context
                            .persist_prepared_payload_segments(session, prepared)
                            .map_err(control_error_to_recording)
                    },
                )
                .map_err(recording_error_to_control);
            result
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
        self.persist_llm_pipeline_diagnostics_fail_local(trace_runtime, llm_pipeline_diagnostics);
        result?;
        self.persist_mcp_stdio_diagnostics_impl(trace_runtime, mcp_stdio_diagnostics)
    }
}

struct PayloadTransactionContext<'a> {
    trace_runtime: &'a TraceRuntime,
    payload_body_retention_gate: &'a mut PayloadBodyRetentionGate,
    semantic_retention: &'a SemanticRetentionConfig,
    application_protocol: &'a mut ApplicationProtocolAnalyzer,
    semantic_actions: &'a mut LiveSemanticActionRuntime,
    mcp_stdio_diagnostics: &'a mut Vec<LiveMcpStdioDiagnostic>,
    llm_pipeline_diagnostics: &'a mut Vec<LlmPipelineDiagnostic>,
    finalized_terminal_traces: &'a mut BTreeSet<TraceId>,
    workload_diagnostics: &'a WorkloadDiagnostics,
    retained_payload_bytes_by_trace: &'a mut BTreeMap<TraceId, u64>,
    retained_payload_transaction: &'a mut RetainedPayloadTransaction,
    next_event_id: &'a mut u64,
    next_payload_segment_id: &'a mut u64,
    diagnostic_log_level: DiagnosticLogLevel,
    policy: PayloadPolicyConfig,
}

enum PreparedPayloadSegment {
    Retained {
        stored_segment: PayloadSegment,
        semantic_actions: SemanticActionBatch,
        application_events: Vec<PreparedApplicationEvent>,
        next_retained_bytes: u64,
        retained_body_bytes: u64,
    },
    SemanticOnly {
        segment: PayloadSegment,
        semantic_actions: SemanticActionBatch,
        application_events: Vec<PreparedApplicationEvent>,
    },
}

struct PreparedApplicationEvent {
    event: DomainEvent,
    semantic_actions: SemanticActionBatch,
}

impl PayloadTransactionContext<'_> {
    fn prepare_payload_segments(
        &mut self,
        storage: &dyn StorageBackend,
        raw_segments: Vec<ResolvedRawPayloadSegment>,
        export_batch: &mut SemanticActionBatch,
    ) -> Result<Vec<PreparedPayloadSegment>, ControlError> {
        let mut prepared = Vec::new();
        for raw in raw_segments {
            if let Some(segment) = self.prepare_payload_segment(storage, raw, export_batch)? {
                prepared.push(segment);
            }
        }
        Ok(prepared)
    }

    fn prepare_payload_segment(
        &mut self,
        storage: &dyn StorageBackend,
        resolved: ResolvedRawPayloadSegment,
        export_batch: &mut SemanticActionBatch,
    ) -> Result<Option<PreparedPayloadSegment>, ControlError> {
        let discontinuity_before = resolved.discontinuity_before;
        let raw = resolved.raw;
        let Some(membership) = self
            .trace_runtime
            .find_membership_in_trace(raw.trace_id, &resolved.process)
        else {
            self.log_payload_diagnostic(format_args!(
                "payload_persist drop_membership_miss trace_id={} process_id={} source={:?} operation_id={}",
                raw.trace_id,
                resolved.process.get(),
                raw.source_boundary,
                raw.operation_id
            ));
            return Ok(None);
        };
        let policy = self.policy.for_segment(&raw)?;
        let stdio_dropped = policy.stdio_storage_mode == PayloadStdioStorageMode::Drop;
        let retain_payload_segment = !stdio_dropped && self.semantic_retention.l4_payload.enabled;
        let mut segment = PayloadSegment {
            // Persisted allocation fails before this semantic-only evidence sentinel.
            segment_id: PayloadSegmentId::new(u64::MAX),
            trace_id: raw.trace_id,
            observed_at: raw.observed_at,
            process: membership.identity,
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
        if stdio_dropped
            && !self
                .semantic_actions
                .should_project_dropped_stdio_payload(&segment)
        {
            self.log_payload_diagnostic(format_args!(
                "payload_persist drop_stdio_storage_policy trace_id={} process_id={} stream={} operation_id={}",
                segment.trace_id,
                segment.process.get(),
                segment.protocol_hint.as_deref().unwrap_or("unknown"),
                segment.operation_id
            ));
            return Ok(None);
        }
        if !stdio_dropped {
            segment.segment_id = self.next_payload_segment_id()?;
        }
        self.mark_semantic_projection_dirty(segment.trace_id);
        let (bytes, redaction) =
            redact_payload_bytes(policy.redaction, std::mem::take(&mut segment.bytes));
        segment.redaction = redaction;
        segment.bytes = bytes;
        let stream_identity = PayloadStreamIdentity::from_segment(&segment);
        let operation_capture_ended = segment
            .operation_offset
            .saturating_add(segment.captured_size)
            >= segment.operation_captured_size;
        let operation_incomplete = operation_capture_ended
            && (segment.truncation == model_core::payload::PayloadTruncationState::Truncated
                || matches!(
                    segment.operation_completion_state,
                    model_core::payload::PayloadOperationCompletionState::Partial
                        | model_core::payload::PayloadOperationCompletionState::Failed
                )
                || segment.operation_original_size != segment.operation_captured_size);
        let incomplete_http1_message = (operation_incomplete && !discontinuity_before)
            .then(|| self.application_protocol.incomplete_http1_message(&segment))
            .flatten();
        if discontinuity_before || operation_incomplete {
            self.application_protocol.forget_stream(&stream_identity);
            self.payload_body_retention_gate
                .forget_stream(&stream_identity);
        }
        let mut boundary_semantic_actions = SemanticActionBatch::default();
        if discontinuity_before {
            boundary_semantic_actions.extend(self.observe_payload_gap(&segment));
        } else if operation_incomplete {
            match incomplete_http1_message {
                Some(IncompleteHttp1Message::Request {
                    sequence,
                    header_projected,
                }) => self.semantic_actions.prepare_incomplete_http1_request(
                    &segment,
                    sequence,
                    header_projected,
                ),
                Some(IncompleteHttp1Message::Response {
                    sequence,
                    header_projected,
                }) => {
                    tracing::warn!(
                        trace_id = segment.trace_id.get(),
                        process_id = segment.process.get(),
                        stream_key = %segment.stream_key,
                        message_sequence = sequence,
                        terminal_sequence = segment.operation_id,
                        "localized incomplete HTTP/1 response to one message"
                    );
                    boundary_semantic_actions.extend(self.prepare_incomplete_http1_response(
                        &segment,
                        sequence,
                        header_projected,
                    ));
                }
                None => self.semantic_actions.prepare_incomplete_payload(&segment),
            }
        }
        if stdio_dropped {
            let started = crate::services::workload_diagnostics::now();
            let mut semantic_actions = boundary_semantic_actions;
            semantic_actions.extend(self.observe_payload_semantics(&segment, false));
            if operation_incomplete {
                let finalized = match incomplete_http1_message {
                    Some(IncompleteHttp1Message::Response { .. }) => {
                        self.finish_incomplete_http1_response(&segment)
                    }
                    Some(IncompleteHttp1Message::Request { .. }) => Default::default(),
                    _ => self.finish_incomplete_payload(&segment),
                };
                semantic_actions.extend(finalized);
            }
            export_batch.extend(semantic_actions.clone());
            self.workload_diagnostics.record_payload_transaction_phase(
                PayloadTransactionPhase::SemanticObserve,
                started.elapsed(),
                semantic_actions.actions().len(),
            );
            return Ok(Some(PreparedPayloadSegment::SemanticOnly {
                segment,
                semantic_actions,
                application_events: Vec::new(),
            }));
        }
        let body_retention: PayloadBodyRetentionDecision = if operation_incomplete {
            PayloadBodyRetentionGate::discontinuity_decision()
        } else {
            self.payload_body_retention_gate.decide(&segment)
        };
        let analysis_segment = segment.clone();
        let retained_payload = if retain_payload_segment {
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
            let retained_bytes = self.retained_payload_bytes(storage, raw.trace_id)?;
            let next_retained_bytes =
                retained_bytes
                    .checked_add(retained_body_bytes)
                    .ok_or_else(|| {
                        ControlError::new("payload_retention", "payload retention overflow")
                    })?;
            if next_retained_bytes > policy.retention_max_bytes_per_trace {
                return Err(ControlError::new(
                    "payload_retention",
                    format!(
                        "trace {} payload retention would exceed configured maximum {} bytes",
                        raw.trace_id, policy.retention_max_bytes_per_trace
                    ),
                ));
            }
            self.workload_diagnostics.record_payload_transaction_phase(
                PayloadTransactionPhase::RetentionCheck,
                started.elapsed(),
                0,
            );
            Some((stored_segment, next_retained_bytes, retained_body_bytes))
        } else {
            None
        };
        let started = crate::services::workload_diagnostics::now();
        let mut semantic_actions = boundary_semantic_actions;
        semantic_actions
            .extend(self.observe_payload_semantics(&analysis_segment, retain_payload_segment));
        if operation_incomplete {
            let finalized = match incomplete_http1_message {
                Some(IncompleteHttp1Message::Response { .. }) => {
                    self.finish_incomplete_http1_response(&analysis_segment)
                }
                Some(IncompleteHttp1Message::Request { .. }) => Default::default(),
                _ => self.finish_incomplete_payload(&analysis_segment),
            };
            semantic_actions.extend(finalized);
        }
        export_batch.extend(semantic_actions.clone());
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::SemanticObserve,
            started.elapsed(),
            semantic_actions.actions().len(),
        );
        let started = crate::services::workload_diagnostics::now();
        let mut application_drafts =
            if application_protocol_requested(self.trace_runtime, raw.trace_id)? {
                let result = if operation_incomplete {
                    self.application_protocol
                        .analyze_incomplete_http1_head_with_semantic_context(
                            &analysis_segment,
                            body_retention.semantic_layer.consumed_by_llm(),
                        )
                } else {
                    self.application_protocol.analyze_with_semantic_context(
                        &analysis_segment,
                        body_retention.semantic_layer.consumed_by_llm(),
                        matches!(body_retention.mode, PayloadBodyRetention::SummaryOnly),
                    )
                };
                result.map_err(|error| ControlError::new("application_protocol_analyzer", error))?
            } else {
                Vec::new()
            };
        if let Some(summary) = tls_summary_application_draft(&analysis_segment) {
            application_drafts.push(summary);
        }
        let application_draft_count = application_drafts.len();
        self.workload_diagnostics.record_payload_transaction_phase(
            PayloadTransactionPhase::ApplicationAnalyze,
            started.elapsed(),
            application_draft_count,
        );
        let application_events = self.prepare_application_events(
            raw.trace_id,
            raw.observed_at,
            membership.identity,
            application_drafts,
            export_batch,
        )?;
        let transaction_tail = self.finish_llm_transaction(&analysis_segment);
        export_batch.extend(transaction_tail.clone());
        semantic_actions.extend(transaction_tail);
        self.payload_body_retention_gate
            .apply(&analysis_segment, body_retention);

        let prepared = if let Some((stored_segment, next_retained_bytes, retained_body_bytes)) =
            retained_payload
        {
            self.retained_payload_transaction
                .record_persisted(raw.trace_id, next_retained_bytes);
            PreparedPayloadSegment::Retained {
                stored_segment,
                semantic_actions,
                application_events,
                next_retained_bytes,
                retained_body_bytes,
            }
        } else {
            PreparedPayloadSegment::SemanticOnly {
                segment: analysis_segment,
                semantic_actions,
                application_events,
            }
        };
        Ok(Some(prepared))
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
        storage: &dyn StorageBackend,
        trace_id: TraceId,
    ) -> Result<u64, ControlError> {
        self.retained_payload_transaction.bytes(
            self.retained_payload_bytes_by_trace,
            storage,
            trace_id,
        )
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
}

struct ResolvedRawPayloadSegment {
    raw: RawPayloadSegment,
    process: ProcessIdentity,
    discontinuity_before: bool,
}

fn recording_error_to_control(error: RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn control_error_to_recording(error: ControlError) -> RecordingError {
    RecordingError::new(error.code, error.message)
}

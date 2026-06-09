//! Payload segment policy and persistence.

use config_core::daemon::{DiagnosticLogLevel, PayloadRedactionPolicy};
use control_contract::reply::ControlError;
use model_core::capability::{Capability, RequestMode};
use model_core::event::{DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload};
use model_core::ids::{CollectorName, TraceId};
use model_core::payload::{
    PayloadRedactionState, PayloadSegment, PayloadSegmentId, PayloadSourceBoundary,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use store_read_contract::payloads::PayloadReadStore;
use store_tx_contract::boundary::TransactionBoundary;
use store_write_contract::events::EventWriteStore;
use store_write_contract::payloads::PayloadWriteStore;
use trace_runtime::registry::TraceRuntime;

use crate::services::application_protocol::{
    ApplicationEventDraft, COLLECTOR_NAME as APPLICATION_PROTOCOL_COLLECTOR_NAME,
};
use crate::services::attach::SqliteAttachService;
use crate::services::identity::TraceIdentityResolver;
use crate::services::payload_gate::{PayloadBodyRetention, PayloadBodyRetentionDecision};
use crate::services::semantic_actions::SemanticActionBatch;

impl SqliteAttachService {
    #[cfg(test)]
    pub(super) fn process_payload_segment_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw: RawPayloadSegment,
    ) -> Result<(), ControlError> {
        self.process_payload_segments_impl(trace_runtime, vec![raw])
    }

    pub(super) fn process_payload_segments_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw_segments: Vec<RawPayloadSegment>,
    ) -> Result<(), ControlError> {
        if raw_segments.is_empty() {
            return Ok(());
        }
        let transaction = self
            .storage
            .begin()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let write_result =
            self.process_payload_segments_in_transaction(trace_runtime, raw_segments);
        match write_result {
            Ok(semantic_actions) => {
                transaction
                    .commit()
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                self.publish_live_otel_action_batch(trace_runtime, &semantic_actions)
            }
            Err(error) => {
                let _ = transaction.rollback();
                Err(error)
            }
        }
    }

    fn process_payload_segments_in_transaction(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw_segments: Vec<RawPayloadSegment>,
    ) -> Result<SemanticActionBatch, ControlError> {
        let mut semantic_actions = SemanticActionBatch::default();
        for raw in raw_segments {
            semantic_actions.extend(self.process_admitted_payload_segment(trace_runtime, raw)?);
        }
        Ok(semantic_actions)
    }

    fn process_admitted_payload_segment(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw: RawPayloadSegment,
    ) -> Result<SemanticActionBatch, ControlError> {
        let admitted = self
            .socket_payload_gate
            .admit(raw)
            .map_err(|error| ControlError::new("socket_payload_gate", error))?;
        let mut semantic_actions = SemanticActionBatch::default();
        for raw in admitted {
            semantic_actions.extend(self.persist_payload_segment(trace_runtime, raw)?);
        }
        Ok(semantic_actions)
    }

    fn persist_payload_segment(
        &mut self,
        trace_runtime: &TraceRuntime,
        raw: RawPayloadSegment,
    ) -> Result<SemanticActionBatch, ControlError> {
        let Some(matched) = TraceIdentityResolver::new(trace_runtime).payload_process(&raw) else {
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

        let policy = self.payload_policy(raw.source_boundary)?;
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
        let analysis_segment = segment.clone();
        match body_retention.mode {
            PayloadBodyRetention::Full => {
                let (bytes, redaction) =
                    redact_payload_bytes(policy.redaction, std::mem::take(&mut segment.bytes));
                segment.captured_size = bytes.len() as u64;
                segment.redaction = redaction;
                segment.bytes = bytes;
            }
            PayloadBodyRetention::SummaryOnly => {
                segment.bytes.clear();
                segment.captured_size = 0;
            }
        }
        let retained_bytes = self
            .storage
            .retained_payload_bytes(raw.trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let next_retained_bytes = retained_bytes
            .checked_add(segment.captured_size)
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
        self.storage
            .append_payload_segment(segment.clone())
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.log_payload_diagnostic(format_args!(
            "payload_persist stored trace_id={} pid={} generation={} source={:?} bytes={} operation_id={}",
            raw.trace_id,
            matched.process.pid,
            matched.process.generation,
            raw.source_boundary,
            segment.captured_size,
            raw.operation_id
        ));
        let mut semantic_actions = self.observe_semantic_actions_for_payload_segment(&segment);
        self.write_semantic_action_batch(&semantic_actions)?;
        let application_drafts = if application_protocol_requested(trace_runtime, raw.trace_id)? {
            match body_retention.mode {
                PayloadBodyRetention::Full => self
                    .application_protocol
                    .analyze(&analysis_segment)
                    .map_err(|error| ControlError::new("application_protocol_analyzer", error))?,
                PayloadBodyRetention::SummaryOnly => self
                    .application_protocol
                    .analyze_summary_only(&analysis_segment)
                    .map_err(|error| ControlError::new("application_protocol_analyzer", error))?,
            }
        } else {
            Vec::new()
        };
        semantic_actions.extend(self.persist_application_events(
            raw.trace_id,
            raw.observed_at,
            matched.process,
            application_drafts,
        )?);
        self.payload_body_retention_gate
            .apply(&analysis_segment, body_retention);
        Ok(semantic_actions)
    }

    fn next_payload_segment_id(&mut self) -> Result<PayloadSegmentId, ControlError> {
        let raw = self.next_payload_segment_id;
        self.next_payload_segment_id =
            self.next_payload_segment_id.checked_add(1).ok_or_else(|| {
                ControlError::new("payload_segment_id_overflow", "payload segment id overflow")
            })?;
        Ok(PayloadSegmentId::new(raw))
    }

    fn payload_policy(
        &self,
        source_boundary: PayloadSourceBoundary,
    ) -> Result<PayloadProcessingPolicy, ControlError> {
        match source_boundary {
            PayloadSourceBoundary::TlsUserSpace => {
                if !self.payload_tls_enabled {
                    return Err(ControlError::new(
                        "payload_policy",
                        "TLS payload segment received while payload_tls_enabled=false",
                    ));
                }
                Ok(PayloadProcessingPolicy {
                    redaction: self.payload_tls_redaction_policy,
                    retention_max_bytes_per_trace: self.payload_tls_retention_max_bytes_per_trace,
                })
            }
            PayloadSourceBoundary::Stdio => {
                if !self.payload_stdio_enabled {
                    return Err(ControlError::new(
                        "payload_policy",
                        "stdio payload segment received while payload_stdio_enabled=false",
                    ));
                }
                Ok(PayloadProcessingPolicy {
                    redaction: self.payload_stdio_redaction_policy,
                    retention_max_bytes_per_trace: self.payload_stdio_retention_max_bytes_per_trace,
                })
            }
            PayloadSourceBoundary::Syscall => {
                if !self.payload_socket_enabled {
                    return Err(ControlError::new(
                        "payload_policy",
                        "socket payload segment received while payload_socket_enabled=false",
                    ));
                }
                Ok(PayloadProcessingPolicy {
                    redaction: self.payload_socket_redaction_policy,
                    retention_max_bytes_per_trace: self
                        .payload_socket_retention_max_bytes_per_trace,
                })
            }
        }
    }

    fn persist_application_events(
        &mut self,
        trace_id: TraceId,
        observed_at: std::time::SystemTime,
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
            self.storage
                .append_event(event.clone())
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            let event_actions = self.observe_semantic_actions_for_event(&event);
            self.write_semantic_action_batch(&event_actions)?;
            semantic_actions.extend(event_actions);
        }
        Ok(semantic_actions)
    }

    fn log_payload_diagnostic(&self, args: std::fmt::Arguments<'_>) {
        self.log_diagnostic(DiagnosticLogLevel::Debug, args);
    }
}

struct PayloadProcessingPolicy {
    redaction: PayloadRedactionPolicy,
    retention_max_bytes_per_trace: u64,
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

fn redact_payload_bytes(
    policy: PayloadRedactionPolicy,
    bytes: Vec<u8>,
) -> (Vec<u8>, PayloadRedactionState) {
    match policy {
        PayloadRedactionPolicy::Disabled => (bytes, PayloadRedactionState::Unredacted),
        PayloadRedactionPolicy::AuthorizationHeader => redact_authorization_header(bytes),
    }
}

fn redact_authorization_header(bytes: Vec<u8>) -> (Vec<u8>, PayloadRedactionState) {
    let mut output = Vec::with_capacity(bytes.len());
    let mut changed = false;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let without_newline = line.strip_suffix(b"\n").unwrap_or(line);
        let without_crlf = without_newline
            .strip_suffix(b"\r")
            .unwrap_or(without_newline);
        if starts_with_ignore_ascii_case(without_crlf, b"authorization:") {
            output.extend_from_slice(b"Authorization: <redacted>");
            if line.ends_with(b"\r\n") {
                output.extend_from_slice(b"\r\n");
            } else if line.ends_with(b"\n") {
                output.push(b'\n');
            }
            changed = true;
        } else {
            output.extend_from_slice(line);
        }
    }

    if changed {
        (output, PayloadRedactionState::Redacted)
    } else {
        (output, PayloadRedactionState::Unredacted)
    }
}

fn starts_with_ignore_ascii_case(value: &[u8], prefix: &[u8]) -> bool {
    value.len() >= prefix.len()
        && value[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

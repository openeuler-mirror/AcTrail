//! Connection and logical-stream routing with bounded transport assembly.

use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::payload::{
    PayloadContentState, PayloadOperationCompletionState, PayloadSegment, PayloadSourceBoundary,
    PayloadTruncationState,
};
use semantic_action::{SemanticAction, SemanticActionKind};

use crate::llm_pipeline::assembly::http2::Http2ConnectionAssembly;
use crate::llm_pipeline::assembly::plain::PlainStreamAssembly;
use crate::llm_pipeline::projection::ProjectionBatch as LiveLlmOutput;
use crate::llm_pipeline::projection::projector::ProjectedProviderResponseId;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::{ResponseFinalizer, StreamFinalizationReason};
use crate::llm_pipeline::transport::http1::Http1DecodeFailure;
use crate::llm_pipeline::transport::http2::{HTTP2_CONNECTION_PREFACE, decode_http2_frame};

use super::identity::{LiveStreamDirection, LiveStreamKey, PayloadStreamGroupKey};
use super::limits::{AssemblyLimits, AssemblyResetReason};
use super::recovery::{RecoveryBoundary, RecoveryScanner};

/// The byte-stream assembly for one (stream_key, direction): either a plain
/// sequential stream (HTTP/1, raw) or a de-multiplexed HTTP/2 connection.
pub(super) enum StreamBody {
    Plain(PlainStreamAssembly),
    Http2(Http2ConnectionAssembly),
}

#[derive(Clone, Copy)]
enum ResetTransport {
    Http1,
    Http2,
}

impl ResetTransport {
    const fn diagnostic_stage(self) -> LlmPipelineDiagnosticStage {
        match self {
            Self::Http1 => LlmPipelineDiagnosticStage::Http1,
            Self::Http2 => LlmPipelineDiagnosticStage::Http2,
        }
    }
}

#[derive(Clone, Copy)]
struct ResetContext {
    reason: AssemblyResetReason,
    skipped_bytes: usize,
    http1_decode_failure: Option<Http1DecodeFailure>,
}

impl ResetContext {
    fn discontinuity(reason: AssemblyResetReason, skipped_bytes: usize) -> Self {
        Self {
            reason,
            skipped_bytes,
            http1_decode_failure: None,
        }
    }

    fn http1_decode(failure: Http1DecodeFailure, skipped_bytes: usize) -> Self {
        Self {
            reason: AssemblyResetReason::ProtocolDecodeFailed,
            skipped_bytes,
            http1_decode_failure: Some(failure),
        }
    }

    fn diagnostic_code(self, transport: ResetTransport) -> LlmPipelineDiagnosticCode {
        match transport {
            ResetTransport::Http1 => match self.http1_decode_failure {
                Some(Http1DecodeFailure::InvalidHead) => {
                    LlmPipelineDiagnosticCode::Http1InvalidHead
                }
                Some(Http1DecodeFailure::InvalidContentLength) => {
                    LlmPipelineDiagnosticCode::Http1InvalidContentLength
                }
                Some(Http1DecodeFailure::InvalidChunkSize) => {
                    LlmPipelineDiagnosticCode::Http1InvalidChunkSize
                }
                Some(Http1DecodeFailure::InvalidChunkTerminator) => {
                    LlmPipelineDiagnosticCode::Http1InvalidChunkTerminator
                }
                Some(Http1DecodeFailure::BufferCapacity) => {
                    LlmPipelineDiagnosticCode::Http1DecoderBufferCapacityExceeded
                }
                None => match self.reason {
                    AssemblyResetReason::ProtocolDecodeFailed => {
                        LlmPipelineDiagnosticCode::Http1ProtocolDecodeFailed
                    }
                    AssemblyResetReason::BufferBytesExceeded => {
                        LlmPipelineDiagnosticCode::Http1AssemblyBufferCapacityExceeded
                    }
                    AssemblyResetReason::SegmentRangesExceeded => {
                        LlmPipelineDiagnosticCode::Http1AssemblySegmentCapacityExceeded
                    }
                    AssemblyResetReason::ConfirmedGap => {
                        LlmPipelineDiagnosticCode::Http1ConfirmedGap
                    }
                    AssemblyResetReason::OperationIncomplete => {
                        LlmPipelineDiagnosticCode::Http1OperationIncomplete
                    }
                    AssemblyResetReason::Http2StreamReset => {
                        LlmPipelineDiagnosticCode::Http1TransportReset
                    }
                },
            },
            ResetTransport::Http2 => match self.reason {
                AssemblyResetReason::ProtocolDecodeFailed => {
                    LlmPipelineDiagnosticCode::Http2ProtocolDecodeFailed
                }
                AssemblyResetReason::Http2StreamReset => {
                    LlmPipelineDiagnosticCode::Http2PeerStreamReset
                }
                AssemblyResetReason::BufferBytesExceeded => {
                    LlmPipelineDiagnosticCode::Http2AssemblyBufferCapacityExceeded
                }
                AssemblyResetReason::SegmentRangesExceeded => {
                    LlmPipelineDiagnosticCode::Http2AssemblySegmentCapacityExceeded
                }
                AssemblyResetReason::ConfirmedGap => LlmPipelineDiagnosticCode::Http2ConfirmedGap,
                AssemblyResetReason::OperationIncomplete => {
                    LlmPipelineDiagnosticCode::Http2OperationIncomplete
                }
            },
        }
    }
}

pub(in crate::llm_pipeline) struct LiveStreamState {
    body: StreamBody,
    desynchronized: bool,
    desynchronized_discarded_bytes: u64,
    desynchronized_discarded_entries: u64,
    recovery_scanner: RecoveryScanner,
}

impl Default for LiveStreamState {
    fn default() -> Self {
        Self {
            body: StreamBody::Plain(PlainStreamAssembly::default()),
            desynchronized: false,
            desynchronized_discarded_bytes: 0,
            desynchronized_discarded_entries: 0,
            recovery_scanner: RecoveryScanner::default(),
        }
    }
}

impl LiveStreamState {
    pub(in crate::llm_pipeline) fn observe_segment(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        limits: AssemblyLimits,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
    ) -> LiveLlmOutput {
        let incomplete_reason = incomplete_segment_reason(segment);
        let mut recovered_discard = LiveLlmOutput::default();
        let mut segment_already_appended = false;
        if self.desynchronized {
            let admission_failure = match &self.body {
                StreamBody::Plain(plain) => plain.admission_failure(segment.bytes.len(), 1, limits),
                StreamBody::Http2(_) => None,
            };
            if let Some(reason) = admission_failure {
                return self.reset_for_discontinuity(
                    config,
                    codecs,
                    key,
                    segment,
                    reason,
                    segment_original_bytes(segment),
                );
            }
            let StreamBody::Plain(plain) = &mut self.body else {
                unreachable!("desynchronized stream recovery always owns a plain assembly")
            };
            plain.append_segment(segment);
            segment_already_appended = true;
            loop {
                let boundary = {
                    let StreamBody::Plain(plain) = &self.body else {
                        unreachable!("desynchronized stream recovery always owns a plain assembly")
                    };
                    if plain.buffer.starts_with(HTTP2_CONNECTION_PREFACE) {
                        RecoveryBoundary::Found(0)
                    } else if HTTP2_CONNECTION_PREFACE.starts_with(&plain.buffer) {
                        RecoveryBoundary::NeedMore
                    } else {
                        self.recovery_scanner.inspect(key.direction, &plain.buffer)
                    }
                };
                match boundary {
                    RecoveryBoundary::NeedMore => {
                        if let Some(reason) = incomplete_reason {
                            let missing_bytes =
                                segment_original_bytes(segment).saturating_sub(segment.bytes.len());
                            return self.reset_for_discontinuity(
                                config,
                                codecs,
                                key,
                                segment,
                                reason,
                                missing_bytes,
                            );
                        }
                        return LiveLlmOutput::default();
                    }
                    RecoveryBoundary::Found(encoded_len) => {
                        if encoded_len != 0 {
                            self.discard_recovery_prefix(encoded_len);
                        }
                        break;
                    }
                }
            }
            self.append_desynchronized_discard_diagnostic(
                &mut recovered_discard,
                key,
                segment.observed_at,
            );
            self.desynchronized = false;
            self.recovery_scanner.reset();
            tracing::warn!(
                trace_id = key.group.trace_id.get(),
                process_id = key.group.process.get(),
                stream_key = %key.group.stream_key,
                direction = ?key.direction,
                "LLM plaintext assembly resynchronized at a trusted HTTP boundary"
            );
        }
        let mut output = match &mut self.body {
            StreamBody::Plain(plain) => {
                if !segment_already_appended {
                    if let Some(reason) = plain.admission_failure(segment.bytes.len(), 1, limits) {
                        return self.reset_for_discontinuity(
                            config,
                            codecs,
                            key,
                            segment,
                            reason,
                            segment_original_bytes(segment),
                        );
                    }
                    plain.append_segment(segment);
                }
                if looks_like_http2(&plain.buffer) {
                    let resets = self.activate_http2(limits);
                    log_http2_stream_resets(key, &resets);
                    let mut output = match &mut self.body {
                        StreamBody::Http2(http2) => {
                            http2.project(config, codecs, &key.group, key.direction)
                        }
                        StreamBody::Plain(_) => unreachable!(),
                    };
                    append_http2_reset_diagnostic(&mut output, key, segment.observed_at, &resets);
                    output
                } else {
                    let mut output = match key.direction {
                        LiveStreamDirection::Outbound => {
                            plain.project_outbound_requests(config, codecs, &key.group)
                        }
                        LiveStreamDirection::Inbound => {
                            plain.project_inbound_responses(config, codecs, &key.group)
                        }
                    };
                    if let Some(failure) = plain.take_http1_decode_failure()
                        && incomplete_reason.is_none()
                    {
                        output.extend(self.reset_for_http1_decode_failure(
                            config, codecs, key, segment, failure, 0,
                        ));
                    }
                    output
                }
            }
            StreamBody::Http2(http2) => {
                if let Some(reason) = http2.admission_failure(segment, limits) {
                    return self.reset_for_discontinuity(
                        config,
                        codecs,
                        key,
                        segment,
                        reason,
                        segment_original_bytes(segment),
                    );
                }
                let resets = http2.append_segment(segment, limits);
                log_http2_stream_resets(key, &resets);
                let mut output = http2.project(config, codecs, &key.group, key.direction);
                append_http2_reset_diagnostic(&mut output, key, segment.observed_at, &resets);
                output
            }
        };
        output.extend(recovered_discard);
        if let Some(reason) = incomplete_reason {
            let missing_bytes = segment_original_bytes(segment).saturating_sub(segment.bytes.len());
            output.extend(self.reset_for_discontinuity(
                config,
                codecs,
                key,
                segment,
                reason,
                missing_bytes,
            ));
        }
        append_codec_diagnostic(&mut output, codecs, key, segment.observed_at);
        output
    }

    /// Convert a plain assembly into an HTTP/2 connection assembly once the
    /// buffered bytes are recognized as HTTP/2 frames.
    fn activate_http2(&mut self, limits: AssemblyLimits) -> Vec<AssemblyResetReason> {
        let StreamBody::Plain(plain) = &mut self.body else {
            return Vec::new();
        };
        let (http2, resets) = Http2ConnectionAssembly::from_plain(plain, limits);
        self.body = StreamBody::Http2(http2);
        resets
    }

    pub(in crate::llm_pipeline) fn reset_for_discontinuity(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
        reason: AssemblyResetReason,
        skipped_bytes: usize,
    ) -> LiveLlmOutput {
        self.reset_with_context(
            config,
            codecs,
            key,
            segment,
            ResetContext::discontinuity(reason, skipped_bytes),
        )
    }

    fn reset_for_http1_decode_failure(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
        failure: Http1DecodeFailure,
        skipped_bytes: usize,
    ) -> LiveLlmOutput {
        self.reset_with_context(
            config,
            codecs,
            key,
            segment,
            ResetContext::http1_decode(failure, skipped_bytes),
        )
    }

    fn reset_with_context(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
        reset: ResetContext,
    ) -> LiveLlmOutput {
        let reason = reset.reason;
        let mut output = LiveLlmOutput::default();
        if key.direction == LiveStreamDirection::Outbound {
            output.extend(self.body.materialize_incomplete_requests(
                config,
                codecs,
                &key.group,
                reason.finalization_reason(),
                segment.observed_at,
            ));
        }
        for (mut actions, drafts, provider_response_ids) in
            self.materialize_in_flight_responses(config, codecs, &key.group)
        {
            output.payload_segments.extend(drafts);
            output.provider_response_ids.extend(provider_response_ids);
            for action in &mut actions {
                if action.kind != SemanticActionKind::LlmResponse {
                    continue;
                }
                ResponseFinalizer::finalize_partial(
                    action,
                    reason.finalization_reason(),
                    segment.observed_at,
                );
                output
                    .non_reusable_response_ids
                    .insert(action.action_id.clone());
            }
            output.actions.extend(actions);
        }
        let buffered_bytes = self.buffered_bytes();
        let transport = match &self.body {
            StreamBody::Plain(_) => ResetTransport::Http1,
            StreamBody::Http2(_) => ResetTransport::Http2,
        };
        let stage = transport.diagnostic_stage();
        let code = reset.diagnostic_code(transport);
        let discarded_bytes = buffered_bytes.saturating_add(reset.skipped_bytes);
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.group.trace_id,
                &key.group.process,
                segment.observed_at,
                code,
                LlmPipelineDiagnosticSeverity::Warning,
                stage,
            )
            .with_stream_key(&key.group.stream_key)
            .with_discarded_bytes(u64::try_from(discarded_bytes).unwrap_or(u64::MAX))
            .with_discarded_entries(1),
        );
        let base_offset = self.stream_end_offset().saturating_add(reset.skipped_bytes);
        self.body = StreamBody::Plain(PlainStreamAssembly::with_base_offset(base_offset));
        self.desynchronized = true;
        self.recovery_scanner.reset();
        tracing::warn!(
            trace_id = key.group.trace_id.get(),
            process_id = key.group.process.get(),
            stream_key = %key.group.stream_key,
            direction = ?key.direction,
            reason = reason.as_str(),
            http1_decode_failure = reset
                .http1_decode_failure
                .map(Http1DecodeFailure::as_str)
                .unwrap_or("none"),
            buffered_bytes,
            "discarded unsafe LLM plaintext assembly state"
        );
        append_codec_diagnostic(&mut output, codecs, key, segment.observed_at);
        output
    }

    fn discard_recovery_prefix(&mut self, encoded_len: usize) {
        let (bytes, entries) = match &self.body {
            StreamBody::Plain(plain) => plain.discarded_prefix_stats(encoded_len),
            StreamBody::Http2(_) => (0, 0),
        };
        self.desynchronized_discarded_bytes =
            self.desynchronized_discarded_bytes.saturating_add(bytes);
        self.desynchronized_discarded_entries = self
            .desynchronized_discarded_entries
            .saturating_add(entries);
        if let StreamBody::Plain(plain) = &mut self.body {
            plain.evict_encoded_len(encoded_len);
        }
    }

    fn append_desynchronized_discard_diagnostic(
        &mut self,
        output: &mut LiveLlmOutput,
        key: &LiveStreamKey,
        observed_at: SystemTime,
    ) {
        if self.desynchronized_discarded_entries == 0 {
            return;
        }
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.group.trace_id,
                &key.group.process,
                observed_at,
                LlmPipelineDiagnosticCode::DesynchronizedBytesDiscarded,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::Lifecycle,
            )
            .with_stream_key(&key.group.stream_key)
            .with_discarded_bytes(self.desynchronized_discarded_bytes)
            .with_discarded_entries(self.desynchronized_discarded_entries),
        );
        self.desynchronized_discarded_bytes = 0;
        self.desynchronized_discarded_entries = 0;
    }

    fn stream_end_offset(&self) -> usize {
        match &self.body {
            StreamBody::Plain(plain) => plain.base_offset.saturating_add(plain.buffer.len()),
            StreamBody::Http2(http2) => http2.end_offset(),
        }
    }

    fn buffered_bytes(&self) -> usize {
        match &self.body {
            StreamBody::Plain(plain) => plain.buffer.len(),
            StreamBody::Http2(http2) => http2.buffered_bytes(),
        }
    }

    pub(in crate::llm_pipeline) fn materialize_closed(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &LiveStreamKey,
        reason: StreamFinalizationReason,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let recovery_bytes = if self.desynchronized {
            match &self.body {
                StreamBody::Plain(plain) => plain.buffer.len(),
                StreamBody::Http2(_) => 0,
            }
        } else {
            0
        };
        if recovery_bytes != 0 {
            self.discard_recovery_prefix(recovery_bytes);
            self.recovery_scanner.reset();
        }
        self.append_desynchronized_discard_diagnostic(&mut output, key, finished_at);
        if key.direction == LiveStreamDirection::Outbound {
            output.extend(self.body.materialize_incomplete_requests(
                config,
                codecs,
                &key.group,
                reason,
                finished_at,
            ));
        }
        if reason == StreamFinalizationReason::PeerClosed
            && key.direction == LiveStreamDirection::Inbound
            && let StreamBody::Plain(plain) = &mut self.body
            && plain.http1_decoder.is_some()
        {
            output
                .extend(plain.project_inbound_responses_with_eof(config, codecs, &key.group, true));
        }
        for (mut actions, drafts, provider_response_ids) in
            self.materialize_in_flight_responses(config, codecs, &key.group)
        {
            output.payload_segments.extend(drafts);
            output.provider_response_ids.extend(provider_response_ids);
            for action in &mut actions {
                if action.kind == SemanticActionKind::LlmResponse {
                    ResponseFinalizer::finalize_partial(action, reason, finished_at);
                }
            }
            output.actions.extend(actions);
        }
        append_codec_diagnostic(&mut output, codecs, key, finished_at);
        output
    }

    /// Materialize every response still in progress when its enclosing trace
    /// closes. HTTP/1 has one sequential message slot; HTTP/2 can have one
    /// independent in-flight response per logical stream.
    fn materialize_in_flight_responses(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> Vec<(
        Vec<SemanticAction>,
        Vec<PayloadSegment>,
        Vec<ProjectedProviderResponseId>,
    )> {
        match &mut self.body {
            StreamBody::Plain(plain) => plain
                .in_flight_response
                .take()
                .and_then(|in_flight| {
                    plain.materialize_in_flight(config, codecs, key, in_flight.message_start)
                })
                .into_iter()
                .collect(),
            StreamBody::Http2(http2) => http2.materialize_in_flight_responses(config, codecs, key),
        }
    }
}

fn incomplete_segment_reason(segment: &PayloadSegment) -> Option<AssemblyResetReason> {
    let operation_capture_ended = segment
        .operation_offset
        .saturating_add(segment.captured_size)
        >= segment.operation_captured_size;
    (operation_capture_ended
        && (segment.truncation == PayloadTruncationState::Truncated
            || matches!(
                segment.operation_completion_state,
                PayloadOperationCompletionState::Partial | PayloadOperationCompletionState::Failed
            )
            || segment.operation_original_size != segment.operation_captured_size))
        .then_some(AssemblyResetReason::OperationIncomplete)
}

fn append_codec_diagnostic(
    output: &mut LiveLlmOutput,
    codecs: &LlmCodecRegistry,
    key: &LiveStreamKey,
    observed_at: SystemTime,
) {
    let failed_plugins = codecs.take_failed_plugin_decodes();
    if failed_plugins == 0 {
        return;
    }
    output.diagnostics.push(
        LlmPipelineDiagnostic::new(
            key.group.trace_id,
            &key.group.process,
            observed_at,
            LlmPipelineDiagnosticCode::ProviderCodecDecodeFailed,
            LlmPipelineDiagnosticSeverity::Warning,
            LlmPipelineDiagnosticStage::ProviderCodec,
        )
        .with_stream_key(&key.group.stream_key)
        .with_discarded_entries(failed_plugins),
    );
}

fn segment_original_bytes(segment: &PayloadSegment) -> usize {
    usize::try_from(segment.original_size).unwrap_or(usize::MAX)
}

fn log_http2_stream_resets(key: &LiveStreamKey, reasons: &[AssemblyResetReason]) {
    for reason in reasons {
        tracing::warn!(
            trace_id = key.group.trace_id.get(),
            process_id = key.group.process.get(),
            stream_key = %key.group.stream_key,
            direction = ?key.direction,
            reason = reason.as_str(),
            "discarded oversized HTTP/2 LLM stream assembly"
        );
    }
}

fn append_http2_reset_diagnostic(
    output: &mut LiveLlmOutput,
    key: &LiveStreamKey,
    observed_at: SystemTime,
    reasons: &[AssemblyResetReason],
) {
    for reason in reasons {
        let code = match reason {
            AssemblyResetReason::ProtocolDecodeFailed => {
                LlmPipelineDiagnosticCode::Http2ProtocolDecodeFailed
            }
            AssemblyResetReason::Http2StreamReset => {
                LlmPipelineDiagnosticCode::Http2PeerStreamReset
            }
            AssemblyResetReason::BufferBytesExceeded => {
                LlmPipelineDiagnosticCode::Http2AssemblyBufferCapacityExceeded
            }
            AssemblyResetReason::SegmentRangesExceeded => {
                LlmPipelineDiagnosticCode::Http2AssemblySegmentCapacityExceeded
            }
            AssemblyResetReason::ConfirmedGap => LlmPipelineDiagnosticCode::Http2ConfirmedGap,
            AssemblyResetReason::OperationIncomplete => {
                LlmPipelineDiagnosticCode::Http2OperationIncomplete
            }
        };
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.group.trace_id,
                &key.group.process,
                observed_at,
                code,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::Http2,
            )
            .with_stream_key(&key.group.stream_key)
            .with_discarded_entries(1),
        );
    }
}

fn looks_like_http2(bytes: &[u8]) -> bool {
    bytes.starts_with(HTTP2_CONNECTION_PREFACE)
        || decode_http2_frame(bytes).is_some_and(|frame| frame.frame_type <= 0x9)
}

pub(in crate::llm_pipeline) fn plaintext_http_candidate(segment: &PayloadSegment) -> bool {
    matches!(
        segment.source_boundary,
        PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
    ) && segment.content_state == PayloadContentState::Plaintext
}

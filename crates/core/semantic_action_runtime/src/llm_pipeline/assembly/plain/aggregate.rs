//! Provider-neutral response aggregate owned by one logical stream.

use std::sync::Arc;

use config_core::daemon::SemanticRetentionConfig;
use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::payload::PayloadSegment;
use semantic_action::SemanticAction;

use crate::llm_pipeline::assembly::router::{
    AssemblyLimits, AssemblyResetReason, PayloadStreamGroupKey,
};
use crate::llm_pipeline::projection::ProjectionBatch as LiveLlmOutput;
use crate::llm_pipeline::projection::projector::{
    InFlightResponse, LiveLlmProjection, ProjectedProviderResponseId, empty_terminal_projection,
    project_decoded_http1_request, project_decoded_http1_response,
    project_raw_llm_response_message,
};
use crate::llm_pipeline::projection::semantic_payload_draft;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::transport::buffer::CursorBuffer;
use crate::llm_pipeline::transport::evidence::{EvidenceCursor, EvidenceSnapshot, EvidenceTracker};
use crate::llm_pipeline::transport::http1::{
    Http1DecodeFailure, Http1Decoder, Http1Direction, RequestBoundary, RequestResynchronizer,
    raw_chunked_candidate, response_candidate_starts_at,
};

use crate::llm_pipeline::stream::finalizer::{ResponseFinalizer, StreamFinalizationReason};
use crate::llm_pipeline::stream::response::IncrementalSseCache;

#[derive(Default)]
pub(in crate::llm_pipeline) struct PlainStreamAssembly {
    pub(in crate::llm_pipeline) buffer: CursorBuffer,
    pub(in crate::llm_pipeline) base_offset: usize,
    pub(in crate::llm_pipeline) segments: EvidenceTracker,
    response_evidence: Option<EvidenceCursor>,
    pub(in crate::llm_pipeline) http1_decoder: Option<Http1Decoder>,
    http1_decode_failure: Option<Http1DecodeFailure>,
    request_resynchronizer: RequestResynchronizer,
    http1_projected_body_len: usize,
    pub(in crate::llm_pipeline) sse_parse_cache: Option<IncrementalSseCache>,
    pub(in crate::llm_pipeline) in_flight_response: Option<InFlightResponse>,
}

impl PlainStreamAssembly {
    pub(in crate::llm_pipeline) fn with_base_offset(base_offset: usize) -> Self {
        Self {
            base_offset,
            ..Self::default()
        }
    }

    pub(in crate::llm_pipeline) fn admission_failure(
        &self,
        appended_bytes: usize,
        appended_ranges: usize,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        if self
            .buffer
            .len()
            .checked_add(appended_bytes)
            .is_none_or(|bytes| bytes > limits.max_buffer_bytes)
        {
            return Some(AssemblyResetReason::BufferBytesExceeded);
        }
        self.segments
            .len()
            .checked_add(appended_ranges)
            .is_none_or(|ranges| ranges > limits.max_segment_ranges)
            .then_some(AssemblyResetReason::SegmentRangesExceeded)
    }

    pub(in crate::llm_pipeline) fn append_segment(&mut self, segment: &PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(&segment.bytes);
        let end = self.base_offset + self.buffer.len();
        self.segments.append(start, end, segment);
    }

    pub(in crate::llm_pipeline) fn discarded_prefix_stats(&self, encoded_len: usize) -> (u64, u64) {
        let end = self.base_offset.saturating_add(encoded_len);
        self.segments.discarded_prefix_stats(end)
    }

    /// Append de-framed plaintext (e.g. one HTTP/2 DATA payload) attributed to
    /// a captured segment.
    pub(in crate::llm_pipeline) fn append_plaintext(
        &mut self,
        bytes: &[u8],
        segments: &[PayloadSegment],
    ) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(bytes);
        let end = self.base_offset + self.buffer.len();
        for segment in segments {
            self.segments.append(start, end, segment);
        }
    }

    pub(in crate::llm_pipeline) fn project_outbound_requests(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        loop {
            if self.http1_decoder.is_none() {
                match self.request_resynchronizer.classify(&self.buffer) {
                    RequestBoundary::Start => {
                        self.http1_decoder = Some(Http1Decoder::new(
                            Http1Direction::Request,
                            usize::try_from(config.l0_llm_call.assembly.max_buffer_bytes)
                                .expect("validated LLM assembly byte limit must fit usize"),
                        ));
                        self.request_resynchronizer.reset();
                    }
                    RequestBoundary::Skip(skip_len) => {
                        self.evict_encoded_len(skip_len);
                        self.request_resynchronizer.reset();
                        continue;
                    }
                    RequestBoundary::NeedMore => break,
                }
            }
            if self.http1_decoder.is_some() {
                let Some(projection) = self.project_decoded_request(config, codecs, key) else {
                    break;
                };
                output.actions.extend(projection.actions);
                output
                    .llm_request_contents
                    .extend(projection.llm_request_contents);
                output
                    .llm_request_histories
                    .extend(projection.llm_request_histories);
                output.llm_tool_results.extend(projection.llm_tool_results);
                output.payload_segments.extend(projection.payload_segments);
                continue;
            }
            break;
        }
        output
    }

    pub(in crate::llm_pipeline) fn project_decoded_request(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> Option<LiveLlmProjection> {
        let message = match self.http1_decoder.as_mut()?.advance(&self.buffer, false) {
            Ok(message) => message?,
            Err(failure) => {
                self.http1_decode_failure = Some(failure);
                return None;
            }
        };
        if !message.complete {
            return None;
        }
        let encoded_len = message.encoded_len;
        let message_start = self.base_offset;
        let segments = self.segments_for_range(message_start, message_start + encoded_len);
        let projection = project_decoded_http1_request(
            config,
            codecs,
            key,
            message_start,
            &self.buffer[..encoded_len],
            message,
            &segments,
        )
        .unwrap_or_else(|| empty_terminal_projection(encoded_len));
        self.evict_encoded_len(encoded_len);
        self.http1_decoder.as_mut()?.reset();
        Some(projection)
    }

    pub(in crate::llm_pipeline) fn materialize_incomplete_request(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        reason: StreamFinalizationReason,
        finished_at: std::time::SystemTime,
    ) -> LiveLlmOutput {
        if self.buffer.is_empty() {
            return LiveLlmOutput::default();
        }
        let buffered_bytes = self.buffer.len();
        let retained_ranges = self.segments.len();
        let message_start = self.base_offset;
        let projection = self
            .http1_decoder
            .as_ref()
            .and_then(Http1Decoder::snapshot)
            .and_then(|message| {
                let encoded_len = message.encoded_len.min(self.buffer.len());
                let segments = self
                    .segments_for_range(message_start, message_start.saturating_add(encoded_len));
                project_decoded_http1_request(
                    config,
                    codecs,
                    key,
                    message_start,
                    &self.buffer[..encoded_len],
                    message,
                    &segments,
                )
            });
        let mut output = LiveLlmOutput::default();
        if let Some(mut projection) = projection
            && !projection.actions.is_empty()
        {
            for action in &mut projection.actions {
                ResponseFinalizer::finalize_partial(action, reason, finished_at);
            }
            output.actions.extend(projection.actions);
            output
                .llm_request_contents
                .extend(projection.llm_request_contents);
            output
                .llm_request_histories
                .extend(projection.llm_request_histories);
            output.llm_tool_results.extend(projection.llm_tool_results);
            output.payload_segments.extend(projection.payload_segments);
            return output;
        }
        let code = if self.http1_decoder.is_some() {
            LlmPipelineDiagnosticCode::Http1IncompleteRequestUnprojectableAtClose
        } else {
            LlmPipelineDiagnosticCode::Http1UnclassifiedBytesDiscardedAtClose
        };
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.trace_id,
                &key.process,
                finished_at,
                code,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::Http1,
            )
            .with_stream_key(&key.stream_key)
            .with_discarded_bytes(buffered_bytes as u64)
            .with_discarded_entries(retained_ranges as u64),
        );
        output
    }

    pub(in crate::llm_pipeline) fn project_inbound_responses(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        self.project_inbound_responses_with_eof(config, codecs, key, false)
    }

    pub(in crate::llm_pipeline) fn project_inbound_responses_with_eof(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        end_of_stream: bool,
    ) -> LiveLlmOutput {
        if self.http1_decoder.is_none() {
            let max_buffer_bytes = usize::try_from(config.l0_llm_call.assembly.max_buffer_bytes)
                .expect("validated LLM assembly byte limit must fit usize");
            if response_candidate_starts_at(&self.buffer) {
                self.http1_decoder = Some(Http1Decoder::new(
                    Http1Direction::Response,
                    max_buffer_bytes,
                ));
            } else if raw_chunked_candidate(&self.buffer) {
                self.http1_decoder = Some(Http1Decoder::new_raw_chunked_response(max_buffer_bytes));
            }
        }
        if self.http1_decoder.is_some() {
            return self.project_decoded_responses(config, codecs, key, end_of_stream);
        }

        let mut output = LiveLlmOutput::default();
        while let Some(projection) = self.project_next_response(config, codecs, key) {
            let terminal = projection.terminal;
            let encoded_len = projection.encoded_len;
            if projection.in_flight.is_some() {
                self.in_flight_response = projection.in_flight;
            } else if terminal || !projection.actions.is_empty() {
                self.in_flight_response = None;
            }
            output.actions.extend(projection.actions);
            output
                .provider_response_ids
                .extend(projection.provider_response_ids);
            output.payload_segments.extend(projection.payload_segments);
            if terminal {
                self.evict_encoded_len(encoded_len);
                self.sse_parse_cache = None;
                self.response_evidence = None;
                if self.buffer.is_empty() {
                    break;
                }
            } else {
                break;
            }
        }
        output
    }

    pub(in crate::llm_pipeline) fn project_decoded_responses(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        end_of_stream: bool,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        loop {
            let decoded = match self
                .http1_decoder
                .as_mut()
                .expect("HTTP/1 decoder exists while projecting")
                .advance(&self.buffer, end_of_stream)
            {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(failure) => {
                    self.http1_decode_failure = Some(failure);
                    break;
                }
            };
            let body_len = decoded.body.len();
            let transport_complete = decoded.complete;
            let encoded_len = decoded.encoded_len;
            if body_len == self.http1_projected_body_len && !transport_complete {
                break;
            }
            let message_start = self.base_offset;
            let mut cache = self.sse_parse_cache.take();
            let Some(evidence) = self.response_evidence(message_start, message_start + encoded_len)
            else {
                break;
            };
            let projection = project_decoded_http1_response(
                config,
                codecs,
                key,
                message_start,
                &self.buffer[..encoded_len],
                decoded,
                &evidence,
                &mut cache,
                transport_complete,
            );
            self.sse_parse_cache = cache;
            self.http1_projected_body_len = body_len;
            if let Some(projection) = projection {
                if projection.in_flight.is_some() {
                    self.in_flight_response = projection.in_flight;
                } else if !projection.actions.is_empty() {
                    self.in_flight_response = None;
                }
                output.actions.extend(projection.actions);
                output
                    .provider_response_ids
                    .extend(projection.provider_response_ids);
                output.payload_segments.extend(projection.payload_segments);
            }
            if !transport_complete {
                break;
            }
            self.evict_encoded_len(encoded_len);
            self.http1_decoder
                .as_mut()
                .expect("HTTP/1 decoder exists after complete message")
                .reset();
            self.http1_projected_body_len = 0;
            self.sse_parse_cache = None;
            self.response_evidence = None;
            self.in_flight_response = None;
            if self.buffer.is_empty() || end_of_stream {
                break;
            }
            if response_candidate_starts_at(&self.buffer) {
                continue;
            }
            if raw_chunked_candidate(&self.buffer) {
                let max_buffer_bytes =
                    usize::try_from(config.l0_llm_call.assembly.max_buffer_bytes)
                        .expect("validated LLM assembly byte limit must fit usize");
                self.http1_decoder = Some(Http1Decoder::new_raw_chunked_response(max_buffer_bytes));
                continue;
            }
            self.http1_decoder = None;
            break;
        }
        output
    }

    pub(in crate::llm_pipeline) fn project_next_response(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> Option<LiveLlmProjection> {
        let mut sse_parse_cache = self.sse_parse_cache.take();
        let encoded_len = self.buffer.len();
        let message_start = self.base_offset;
        let message_end = message_start + encoded_len;
        let evidence = self.response_evidence(message_start, message_end)?;
        let projection = project_raw_llm_response_message(
            config,
            codecs,
            key,
            message_start,
            &self.buffer,
            &evidence,
            &mut sse_parse_cache,
            false,
        );
        self.sse_parse_cache = sse_parse_cache;
        projection
    }

    pub(in crate::llm_pipeline) fn materialize_in_flight(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        message_start: usize,
    ) -> Option<(
        Vec<SemanticAction>,
        Vec<PayloadSegment>,
        Vec<ProjectedProviderResponseId>,
    )> {
        let mut sse_parse_cache = self.sse_parse_cache.take();
        let decoded = self.http1_decoder.as_ref().and_then(Http1Decoder::snapshot);
        let encoded_len = decoded
            .as_ref()
            .map_or_else(|| self.buffer.len(), |message| message.encoded_len);
        let message_end = message_start.checked_add(encoded_len)?;
        let (first, assembled_bytes, projection) = {
            let evidence = self.response_evidence(message_start, message_end)?;
            let first = evidence.first.as_ref()?.clone();
            let assembled_bytes = self.buffer.get(..encoded_len)?.to_vec();
            let projection = match decoded {
                Some(message) => project_decoded_http1_response(
                    config,
                    codecs,
                    key,
                    message_start,
                    &self.buffer[..encoded_len],
                    message,
                    &evidence,
                    &mut sse_parse_cache,
                    true,
                ),
                None => project_raw_llm_response_message(
                    config,
                    codecs,
                    key,
                    message_start,
                    &self.buffer,
                    &evidence,
                    &mut sse_parse_cache,
                    true,
                ),
            };
            (first, assembled_bytes, projection)
        };
        self.sse_parse_cache = sse_parse_cache;
        let payload_segments =
            if config.l4_payload.enabled || !config.l0_llm_call.retain_assembled_payload() {
                Vec::new()
            } else {
                vec![semantic_payload_draft(&first, &assembled_bytes)]
            };
        let projection = projection?;
        Some((
            projection.actions,
            payload_segments,
            projection.provider_response_ids,
        ))
    }

    pub(in crate::llm_pipeline) fn segments_for_range(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<&PayloadSegment> {
        self.segments.for_range(start, end)
    }

    pub(in crate::llm_pipeline) fn response_evidence(
        &mut self,
        message_start: usize,
        message_end: usize,
    ) -> Option<Arc<EvidenceSnapshot>> {
        if self
            .response_evidence
            .as_ref()
            .is_none_or(|cursor| cursor.message_start() != message_start)
        {
            self.response_evidence = Some(self.segments.cursor(message_start));
        }
        let cursor = self.response_evidence.as_mut()?;
        self.segments
            .advance_cursor(cursor, message_end)
            .then(|| cursor.snapshot())
    }

    pub(in crate::llm_pipeline) fn take_http1_decode_failure(
        &mut self,
    ) -> Option<Http1DecodeFailure> {
        self.http1_decode_failure.take()
    }

    pub(in crate::llm_pipeline) fn evict_encoded_len(&mut self, encoded_len: usize) {
        let Some(global_end) = self.base_offset.checked_add(encoded_len) else {
            tracing::warn!(encoded_len, "refused overflowing LLM stream buffer release");
            return;
        };
        if !self.buffer.release(encoded_len) {
            tracing::warn!(
                encoded_len,
                buffered_bytes = self.buffer.len(),
                "refused out-of-range LLM stream buffer release"
            );
            return;
        }
        self.base_offset = global_end;
        self.segments.evict_before(self.base_offset);
        if self.buffer.is_empty() {
            self.segments.reset();
            self.response_evidence = None;
        }
    }
}

//! HTTP/2 connection assembly and logical-stream coordination.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind};

use crate::llm_pipeline::assembly::plain::PlainStreamAssembly;
use crate::llm_pipeline::assembly::router::{
    AssemblyLimits, AssemblyResetReason, LiveStreamDirection, PayloadStreamGroupKey,
};
use crate::llm_pipeline::projection::ProjectionBatch as LiveLlmOutput;
use crate::llm_pipeline::projection::projector::{
    ProjectedProviderResponseId, project_http2_stream_request, project_http2_stream_response,
};
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::ResponseFinalizer;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use crate::llm_pipeline::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};

use crate::llm_pipeline::transport::http2::{Http2DataEvent, Http2Decoder};

/// One sequential plaintext byte stream to assemble and project: a whole
/// HTTP/1 (or raw) connection body, or one de-multiplexed HTTP/2 stream's
/// plaintext (its DATA-frame payloads).
/// One HTTP/2 stream's de-multiplexed plaintext plus its end-of-stream flag.
#[derive(Default)]
pub(in crate::llm_pipeline) struct Http2StreamAssembly {
    pub(in crate::llm_pipeline) plain: PlainStreamAssembly,
    body: Arc<Vec<u8>>,
    pub(in crate::llm_pipeline) end_stream: bool,
    end_stream_observed_at: Option<SystemTime>,
}

impl Http2StreamAssembly {
    fn retention_footprint(&self) -> RetentionFootprint {
        RetentionFootprint {
            bytes: self.plain.buffer.len(),
            ranges: self.plain.segments.len(),
        }
    }

    fn has_confirmed_llm_response(&self) -> bool {
        self.plain.in_flight_response.is_some()
            || self
                .plain
                .sse_parse_cache
                .as_ref()
                .is_some_and(|cache| cache.is_confirmed_llm())
    }

    fn project_request(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        if !self.end_stream || self.plain.buffer.is_empty() {
            return output;
        }
        let message_start = self.plain.base_offset;
        let message_end = message_start + self.plain.buffer.len();
        let segments = self.plain.segments_for_range(message_start, message_end);
        let Some(projection) = project_http2_stream_request(
            config,
            codecs,
            key,
            stream_id,
            message_start,
            &self.plain.buffer,
            Arc::clone(&self.body),
            &segments,
        ) else {
            return output;
        };
        output.actions.extend(projection.actions);
        output
            .llm_request_contents
            .extend(projection.llm_request_contents);
        output
            .llm_request_histories
            .extend(projection.llm_request_histories);
        output.llm_tool_results.extend(projection.llm_tool_results);
        self.plain.evict_encoded_len(projection.encoded_len);
        output
    }

    fn materialize_incomplete_request(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
        reason: StreamFinalizationReason,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        if self.plain.buffer.is_empty() {
            return output;
        }
        let buffered_bytes = self.plain.buffer.len();
        let retained_ranges = self.plain.segments.len();
        let message_start = self.plain.base_offset;
        let message_end = message_start.saturating_add(buffered_bytes);
        let segments = self.plain.segments_for_range(message_start, message_end);
        if let Some(mut projection) = project_http2_stream_request(
            config,
            codecs,
            key,
            stream_id,
            message_start,
            &self.plain.buffer,
            Arc::clone(&self.body),
            &segments,
        ) && !projection.actions.is_empty()
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
        let diagnostic_stream_key = format!("{}#h2:{}", key.stream_key, stream_id);
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.trace_id,
                &key.process,
                finished_at,
                LlmPipelineDiagnosticCode::Http2IncompleteRequestUnprojectableAtClose,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::Http2,
            )
            .with_stream_key(&diagnostic_stream_key)
            .with_discarded_bytes(u64::try_from(buffered_bytes).unwrap_or(u64::MAX))
            .with_discarded_entries(u64::try_from(retained_ranges).unwrap_or(u64::MAX)),
        );
        output
    }

    fn project_response(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        if self.plain.buffer.is_empty() {
            return output;
        }
        let mut sse_parse_cache = self.plain.sse_parse_cache.take();
        let message_start = self.plain.base_offset;
        let message_end = message_start + self.plain.buffer.len();
        let Some(evidence) = self.plain.response_evidence(message_start, message_end) else {
            self.plain.sse_parse_cache = sse_parse_cache;
            return output;
        };
        let Some(projection) = project_http2_stream_response(
            config,
            codecs,
            key,
            stream_id,
            message_start,
            &self.plain.buffer,
            Arc::clone(&self.body),
            &evidence,
            &mut sse_parse_cache,
            self.end_stream,
            self.end_stream,
        ) else {
            self.plain.sse_parse_cache = sse_parse_cache;
            return output;
        };
        self.plain.sse_parse_cache = sse_parse_cache;
        output.actions.extend(projection.actions);
        output
            .provider_response_ids
            .extend(projection.provider_response_ids);
        output.payload_segments.extend(projection.payload_segments);
        self.plain.in_flight_response = projection.in_flight;
        if projection.terminal {
            self.plain.evict_encoded_len(projection.encoded_len);
        }
        output
    }

    pub(in crate::llm_pipeline) fn materialize_response(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
        message_start: usize,
    ) -> Option<(
        Vec<SemanticAction>,
        Vec<PayloadSegment>,
        Vec<ProjectedProviderResponseId>,
    )> {
        if self.plain.buffer.is_empty() {
            return None;
        }
        let mut sse_parse_cache = self.plain.sse_parse_cache.take();
        let message_end = message_start.checked_add(self.plain.buffer.len())?;
        let projection = {
            let evidence = self.plain.response_evidence(message_start, message_end)?;
            project_http2_stream_response(
                config,
                codecs,
                key,
                stream_id,
                message_start,
                &self.plain.buffer,
                Arc::clone(&self.body),
                &evidence,
                &mut sse_parse_cache,
                false,
                true,
            )?
        };
        self.plain.sse_parse_cache = sse_parse_cache;
        Some((
            projection.actions,
            projection.payload_segments,
            projection.provider_response_ids,
        ))
    }
}

/// A whole HTTP/2 connection in one direction: the raw frame byte stream,
/// decomposed into per-stream plaintext assemblies.
pub(in crate::llm_pipeline) struct Http2ConnectionAssembly {
    pub(in crate::llm_pipeline) decoder: Http2Decoder,
    pub(in crate::llm_pipeline) streams: BTreeMap<u32, Http2StreamAssembly>,
    // A stream is scheduled only after DATA mutates its assembly or
    // END_STREAM changes its terminal state. This keeps per-input projection
    // proportional to the streams affected by the decoded frame batch.
    dirty_streams: BTreeSet<u32>,
    discarded_streams: BTreeSet<u32>,
    discard_connection_streams: bool,
    pending_finalizations: Vec<PendingHttp2Finalization>,
    retained_streams: RetainedStreamBudget,
}

#[derive(Clone, Copy, Default)]
struct RetentionFootprint {
    bytes: usize,
    ranges: usize,
}

/// Incremental accounting for plaintext retained by active HTTP/2 streams and
/// streams waiting for fail-local finalization. Decoder retention is accounted
/// separately because frames enter and leave that buffer as one batch.
#[derive(Default)]
struct RetainedStreamBudget {
    bytes: usize,
    ranges: usize,
}

impl RetainedStreamBudget {
    fn admission_failure(
        &self,
        decoder_bytes: usize,
        decoder_ranges: usize,
        appended_bytes: usize,
        appended_ranges: usize,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        if self
            .bytes
            .checked_add(decoder_bytes)
            .and_then(|bytes| bytes.checked_add(appended_bytes))
            .is_none_or(|bytes| bytes > limits.max_buffer_bytes)
        {
            return Some(AssemblyResetReason::BufferBytesExceeded);
        }
        self.ranges
            .checked_add(decoder_ranges)
            .and_then(|ranges| ranges.checked_add(appended_ranges))
            .is_none_or(|ranges| ranges > limits.max_segment_ranges)
            .then_some(AssemblyResetReason::SegmentRangesExceeded)
    }

    fn replace(&mut self, before: RetentionFootprint, after: RetentionFootprint) {
        self.release(before);
        let Some(bytes) = self.bytes.checked_add(after.bytes) else {
            self.fail_closed("byte", self.bytes, after.bytes);
            return;
        };
        let Some(ranges) = self.ranges.checked_add(after.ranges) else {
            self.fail_closed("range", self.ranges, after.ranges);
            return;
        };
        self.bytes = bytes;
        self.ranges = ranges;
    }

    fn release(&mut self, footprint: RetentionFootprint) {
        let Some(bytes) = self.bytes.checked_sub(footprint.bytes) else {
            tracing::error!(
                retained_bytes = self.bytes,
                released_bytes = footprint.bytes,
                "HTTP/2 retained-byte accounting underflow"
            );
            self.bytes = usize::MAX;
            self.ranges = usize::MAX;
            return;
        };
        let Some(ranges) = self.ranges.checked_sub(footprint.ranges) else {
            tracing::error!(
                retained_ranges = self.ranges,
                released_ranges = footprint.ranges,
                "HTTP/2 retained-range accounting underflow"
            );
            self.bytes = usize::MAX;
            self.ranges = usize::MAX;
            return;
        };
        self.bytes = bytes;
        self.ranges = ranges;
    }

    fn fail_closed(&mut self, dimension: &'static str, retained: usize, added: usize) {
        tracing::error!(
            dimension,
            retained,
            added,
            "HTTP/2 retained-stream accounting overflow"
        );
        // Preserve runtime fail-local behavior without weakening the capacity
        // guard: the next admission rejects and resets this connection state.
        self.bytes = usize::MAX;
        self.ranges = usize::MAX;
    }
}

struct PendingHttp2Finalization {
    stream_id: u32,
    stream: Http2StreamAssembly,
    reason: AssemblyResetReason,
    observed_at: SystemTime,
}

impl Default for Http2ConnectionAssembly {
    fn default() -> Self {
        Self {
            decoder: Http2Decoder::default(),
            streams: BTreeMap::new(),
            dirty_streams: BTreeSet::new(),
            discarded_streams: BTreeSet::new(),
            discard_connection_streams: false,
            pending_finalizations: Vec::new(),
            retained_streams: RetainedStreamBudget::default(),
        }
    }
}

impl Http2ConnectionAssembly {
    pub(in crate::llm_pipeline) fn from_plain(
        plain: &mut PlainStreamAssembly,
        limits: AssemblyLimits,
    ) -> (Self, Vec<AssemblyResetReason>) {
        let mut connection = Self {
            decoder: Http2Decoder::from_buffer(
                plain.buffer.take_remaining(),
                plain.base_offset,
                plain.segments.take(),
            ),
            ..Self::default()
        };
        let resets = connection.parse_frames(limits);
        (connection, resets)
    }

    pub(in crate::llm_pipeline) fn end_offset(&self) -> usize {
        self.decoder.end_offset()
    }

    pub(in crate::llm_pipeline) fn buffered_bytes(&self) -> usize {
        self.decoder
            .buffered_bytes()
            .saturating_add(self.retained_streams.bytes)
    }

    pub(in crate::llm_pipeline) fn materialize_in_flight_responses(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> Vec<(
        Vec<SemanticAction>,
        Vec<PayloadSegment>,
        Vec<ProjectedProviderResponseId>,
    )> {
        self.streams
            .iter_mut()
            .filter_map(|(stream_id, stream)| {
                let in_flight = stream.plain.in_flight_response.take()?;
                stream.materialize_response(
                    config,
                    codecs,
                    key,
                    *stream_id,
                    in_flight.message_start,
                )
            })
            .collect()
    }

    pub(in crate::llm_pipeline) fn materialize_incomplete_requests(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        reason: StreamFinalizationReason,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        for (stream_id, stream) in &mut self.streams {
            output.extend(stream.materialize_incomplete_request(
                config,
                codecs,
                key,
                *stream_id,
                reason,
                finished_at,
            ));
        }
        for pending in &mut self.pending_finalizations {
            output.extend(pending.stream.materialize_incomplete_request(
                config,
                codecs,
                key,
                pending.stream_id,
                reason,
                finished_at,
            ));
        }
        output
    }

    pub(in crate::llm_pipeline) fn admission_failure(
        &self,
        segment: &PayloadSegment,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        let decoder_ranges = self
            .decoder
            .evidence_ranges()
            .checked_add(self.discarded_streams.len())
            .unwrap_or(usize::MAX);
        self.retained_streams.admission_failure(
            self.decoder.buffered_bytes(),
            decoder_ranges,
            segment.bytes.len(),
            1,
            limits,
        )
    }

    pub(in crate::llm_pipeline) fn append_segment(
        &mut self,
        segment: &PayloadSegment,
        limits: AssemblyLimits,
    ) -> Vec<AssemblyResetReason> {
        self.decoder.append(segment);
        self.parse_frames(limits)
    }

    pub(in crate::llm_pipeline) fn parse_frames(
        &mut self,
        limits: AssemblyLimits,
    ) -> Vec<AssemblyResetReason> {
        let mut resets = Vec::new();
        let batch = self.decoder.advance();
        for event in batch.data {
            let end_stream = event.end_stream;
            let stream_id = event.stream_id;
            let observed_at = event.evidence.last().map(|segment| segment.observed_at);
            if let Some(reason) = self.route_stream_data(event, limits) {
                resets.push(reason);
            }
            if end_stream {
                self.mark_end_stream(stream_id, observed_at);
            }
        }
        for event in batch.ended {
            self.mark_end_stream(event.stream_id, event.observed_at);
        }
        for event in batch.failures {
            let reason = if event.reset_by_peer {
                AssemblyResetReason::Http2StreamReset
            } else {
                AssemblyResetReason::ProtocolDecodeFailed
            };
            if let Some(stream) = self.streams.remove(&event.stream_id) {
                if let Some(observed_at) = event.observed_at {
                    self.pending_finalizations.push(PendingHttp2Finalization {
                        stream_id: event.stream_id,
                        stream,
                        reason,
                        observed_at,
                    });
                } else {
                    self.retained_streams.release(stream.retention_footprint());
                }
                resets.push(reason);
            }
            self.dirty_streams.remove(&event.stream_id);
            self.discarded_streams.remove(&event.stream_id);
        }
        resets.extend(std::iter::repeat_n(
            AssemblyResetReason::ProtocolDecodeFailed,
            batch.connection_failures,
        ));
        resets
    }

    fn route_stream_data(
        &mut self,
        event: Http2DataEvent,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        let Http2DataEvent {
            stream_id,
            data,
            evidence: segments,
            ..
        } = event;
        if self.discard_connection_streams {
            return None;
        }
        if self.discarded_streams.contains(&stream_id) {
            return None;
        }
        let Some(observed_at) = segments.last().map(|segment| segment.observed_at) else {
            return None;
        };
        let decoder_ranges = self
            .decoder
            .evidence_ranges()
            .checked_add(self.discarded_streams.len())
            .unwrap_or(usize::MAX);
        let global_failure = self.retained_streams.admission_failure(
            self.decoder.buffered_bytes(),
            decoder_ranges,
            data.len(),
            segments.len(),
            limits,
        );
        if let Some(reason) = global_failure {
            self.dirty_streams.remove(&stream_id);
            if let Some(stream) = self.streams.remove(&stream_id) {
                self.pending_finalizations.push(PendingHttp2Finalization {
                    stream_id,
                    stream,
                    reason,
                    observed_at,
                });
            }
            self.remember_discarded_stream(stream_id, limits);
            return Some(reason);
        }
        let stream = self.streams.entry(stream_id).or_default();
        if let Some(reason) = stream
            .plain
            .admission_failure(data.len(), segments.len(), limits)
        {
            self.dirty_streams.remove(&stream_id);
            if let Some(stream) = self.streams.remove(&stream_id) {
                self.pending_finalizations.push(PendingHttp2Finalization {
                    stream_id,
                    stream,
                    reason,
                    observed_at,
                });
            }
            self.remember_discarded_stream(stream_id, limits);
            return Some(reason);
        }
        let before = stream.retention_footprint();
        stream.plain.append_plaintext(&data, &segments);
        Arc::make_mut(&mut stream.body).extend_from_slice(&data);
        self.retained_streams
            .replace(before, stream.retention_footprint());
        self.dirty_streams.insert(stream_id);
        None
    }

    fn remember_discarded_stream(&mut self, stream_id: u32, limits: AssemblyLimits) {
        if self.discarded_streams.len() < limits.max_segment_ranges {
            self.discarded_streams.insert(stream_id);
            return;
        }
        // Tombstones prevent later DATA for a rejected stream from reopening
        // assembly state. Once their configured bound is exhausted, fail this
        // connection locally instead of retaining attacker-controlled stream
        // identifiers without limit.
        self.discarded_streams.clear();
        self.dirty_streams.clear();
        self.discard_connection_streams = true;
    }

    fn mark_end_stream(&mut self, stream_id: u32, observed_at: Option<SystemTime>) {
        if self.discarded_streams.remove(&stream_id) {
            self.dirty_streams.remove(&stream_id);
            return;
        }
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.end_stream = true;
            stream.end_stream_observed_at = observed_at.or(stream.end_stream_observed_at);
            self.dirty_streams.insert(stream_id);
        }
    }

    pub(in crate::llm_pipeline) fn project(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        direction: LiveStreamDirection,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        for mut pending in std::mem::take(&mut self.pending_finalizations) {
            let retained = pending.stream.retention_footprint();
            if direction == LiveStreamDirection::Inbound
                && let Some(in_flight) = pending.stream.plain.in_flight_response.take()
                && let Some((mut actions, drafts, provider_response_ids)) =
                    pending.stream.materialize_response(
                        config,
                        codecs,
                        key,
                        pending.stream_id,
                        in_flight.message_start,
                    )
            {
                output.payload_segments.extend(drafts);
                output.provider_response_ids.extend(provider_response_ids);
                for action in &mut actions {
                    if action.kind == SemanticActionKind::LlmResponse {
                        ResponseFinalizer::finalize_partial(
                            action,
                            pending.reason.finalization_reason(),
                            pending.observed_at,
                        );
                        output
                            .non_reusable_response_ids
                            .insert(action.action_id.clone());
                    }
                }
                output.actions.extend(actions);
            }
            self.retained_streams.release(retained);
        }
        while let Some(stream_id) = self.dirty_streams.pop_first() {
            let Some(stream) = self.streams.get_mut(&stream_id) else {
                continue;
            };
            let before = stream.retention_footprint();
            let was_confirmed_llm_response =
                direction == LiveStreamDirection::Inbound && stream.has_confirmed_llm_response();
            let projected = match direction {
                LiveStreamDirection::Outbound => {
                    stream.project_request(config, codecs, key, stream_id)
                }
                LiveStreamDirection::Inbound => {
                    stream.project_response(config, codecs, key, stream_id)
                }
            };
            let confirmed_llm_response = direction == LiveStreamDirection::Inbound
                && (was_confirmed_llm_response || stream.has_confirmed_llm_response());
            if stream.end_stream && confirmed_llm_response && projection_is_empty(&projected) {
                let observed_at = stream
                    .end_stream_observed_at
                    .unwrap_or_else(SystemTime::now);
                let diagnostic_stream_key = format!("{}#h2:{}", key.stream_key, stream_id);
                output.diagnostics.push(
                    LlmPipelineDiagnostic::new(
                        key.trace_id,
                        &key.process,
                        observed_at,
                        LlmPipelineDiagnosticCode::Http2ConfirmedResponseUnprojectableAtEndStream,
                        LlmPipelineDiagnosticSeverity::Warning,
                        LlmPipelineDiagnosticStage::Http2,
                    )
                    .with_stream_key(&diagnostic_stream_key)
                    .with_discarded_bytes(u64::try_from(before.bytes).unwrap_or(u64::MAX))
                    .with_discarded_entries(u64::try_from(before.ranges).unwrap_or(u64::MAX)),
                );
            }
            output.extend(projected);
            self.retained_streams
                .replace(before, stream.retention_footprint());
            if stream.plain.buffer.is_empty() || stream.end_stream {
                if let Some(stream) = self.streams.remove(&stream_id) {
                    self.retained_streams.release(stream.retention_footprint());
                }
            }
        }
        output
    }
}

fn projection_is_empty(output: &LiveLlmOutput) -> bool {
    output.actions.is_empty()
        && output.llm_request_contents.is_empty()
        && output.llm_request_histories.is_empty()
        && output.llm_tool_results.is_empty()
        && output.provider_response_ids.is_empty()
        && output.payload_segments.is_empty()
        && output.http_request_links.is_empty()
        && output.http_response_links.is_empty()
}

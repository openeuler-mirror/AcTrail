//! HTTP/2 connection assembly and logical-stream coordination.

mod request;

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
    ProjectedProviderResponseId, project_http2_stream_response,
};
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::ResponseFinalizer;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;

/// One sequential plaintext byte stream to assemble and project: a whole
/// HTTP/1 (or raw) connection body, or one de-multiplexed HTTP/2 stream's
/// plaintext (its DATA-frame payloads).
/// One HTTP/2 stream's de-multiplexed plaintext plus its end-of-stream flag.
#[derive(Default)]
pub(in crate::llm_pipeline) struct Http2StreamAssembly {
    discarded: bool,
    semantic_terminal: bool,
    pub(in crate::llm_pipeline) message_id: Option<String>,
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

    fn project_response(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let Some(message_id) = self.message_id.as_deref() else {
            return output;
        };
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
            message_id,
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
                self.message_id.as_deref()?,
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

/// Directional semantic DATA state; framing and stream lifecycle belong to Http2Protocol.
#[derive(Default)]
pub(in crate::llm_pipeline) struct Http2BodyAssembly {
    streams: BTreeMap<u32, Http2StreamAssembly>,
    dirty: BTreeSet<u32>,
    retained_bytes: usize,
    retained_ranges: usize,
}
impl Http2BodyAssembly {
    pub(in crate::llm_pipeline) fn append_data(
        &mut self,
        stream_id: u32,
        message_id: &str,
        data: &[u8],
        evidence: &[PayloadSegment],
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        if self
            .streams
            .get(&stream_id)
            .is_some_and(|stream| stream.discarded || stream.semantic_terminal)
        {
            return None;
        }
        if self
            .retained_bytes
            .checked_add(data.len())
            .is_none_or(|size| size > limits.max_buffer_bytes)
        {
            return Some(AssemblyResetReason::BufferBytesExceeded);
        }
        if self
            .retained_ranges
            .checked_add(evidence.len())
            .is_none_or(|size| size > limits.max_segment_ranges)
        {
            return Some(AssemblyResetReason::SegmentRangesExceeded);
        }
        let stream = self.streams.entry(stream_id).or_default();
        stream.message_id = Some(message_id.to_owned());
        if stream.semantic_terminal {
            return None;
        }
        let before = stream.retention_footprint();
        stream.plain.append_plaintext(data, evidence);
        Arc::make_mut(&mut stream.body).extend_from_slice(data);
        let after = stream.retention_footprint();
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(before.bytes)
            .saturating_add(after.bytes);
        self.retained_ranges = self
            .retained_ranges
            .saturating_sub(before.ranges)
            .saturating_add(after.ranges);
        self.dirty.insert(stream_id);
        None
    }
    pub(in crate::llm_pipeline) fn end_stream(&mut self, stream_id: u32, at: SystemTime) {
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.end_stream = true;
            stream.end_stream_observed_at = Some(at);
            self.dirty.insert(stream_id);
        }
    }
    fn remove(&mut self, stream_id: u32) -> Option<Http2StreamAssembly> {
        self.dirty.remove(&stream_id);
        let stream = self.streams.remove(&stream_id)?;
        let footprint = stream.retention_footprint();
        self.retained_bytes = self.retained_bytes.saturating_sub(footprint.bytes);
        self.retained_ranges = self.retained_ranges.saturating_sub(footprint.ranges);
        Some(stream)
    }
    pub(in crate::llm_pipeline) fn discard_stream(&mut self, stream_id: u32) {
        self.remove(stream_id);
    }
    pub(in crate::llm_pipeline) fn consumer_discarded(&self, stream_id: u32) -> bool {
        self.streams
            .get(&stream_id)
            .is_some_and(|stream| stream.discarded)
    }
    pub(in crate::llm_pipeline) fn discard_consumer(
        &mut self,
        stream_id: u32,
        message_id: &str,
        reason: StreamFinalizationReason,
        at: SystemTime,
        direction: LiveStreamDirection,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        let output = self.abort_stream(stream_id, reason, at, direction, config, codecs, key);
        self.streams.insert(
            stream_id,
            Http2StreamAssembly {
                discarded: true,
                message_id: Some(message_id.to_owned()),
                ..Http2StreamAssembly::default()
            },
        );
        output
    }
    pub(in crate::llm_pipeline) fn abort_stream(
        &mut self,
        stream_id: u32,
        reason: StreamFinalizationReason,
        at: SystemTime,
        direction: LiveStreamDirection,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        let Some(mut stream) = self.remove(stream_id) else {
            return LiveLlmOutput::default();
        };
        if direction == LiveStreamDirection::Outbound {
            return stream
                .materialize_incomplete_request(config, codecs, key, stream_id, reason, at);
        }
        let start = stream.plain.base_offset;
        let Some((mut actions, payload_segments, provider_response_ids)) =
            stream.materialize_response(config, codecs, key, stream_id, start)
        else {
            return LiveLlmOutput::default();
        };
        for action in &mut actions {
            ResponseFinalizer::finalize_incomplete(action, reason, at);
        }
        LiveLlmOutput {
            actions,
            payload_segments,
            provider_response_ids,
            ..LiveLlmOutput::default()
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
        while let Some(stream_id) = self.dirty.pop_first() {
            let Some(stream) = self.streams.get_mut(&stream_id) else {
                continue;
            };
            if stream.discarded {
                if stream.end_stream {
                    self.remove(stream_id);
                }
                continue;
            }
            let before = stream.retention_footprint();
            let was_confirmed =
                direction == LiveStreamDirection::Inbound && stream.has_confirmed_llm_response();
            let projected = match direction {
                LiveStreamDirection::Outbound => {
                    stream.project_request(config, codecs, key, stream_id)
                }
                LiveStreamDirection::Inbound => {
                    stream.project_response(config, codecs, key, stream_id)
                }
            };
            stream.semantic_terminal |= projected.actions.iter().any(|action| {
                action.kind == SemanticActionKind::LlmResponse
                    && action.status != semantic_action::SemanticActionStatus::InProgress
            });
            if stream.semantic_terminal {
                // Keep this stream's transport marker until END_STREAM, but release
                // consumed content before subtracting its retention footprint.
                stream.plain = PlainStreamAssembly::default();
                stream.body = Arc::default();
            }
            if stream.end_stream
                && was_confirmed
                && !stream.semantic_terminal
                && projected.actions.is_empty()
                && let Some(observed_at) = stream.end_stream_observed_at
            {
                output.diagnostics.push(model_core::diagnostics::LlmPipelineDiagnostic::new(
                    key.trace_id, &key.process, observed_at,
                    model_core::diagnostics::LlmPipelineDiagnosticCode::Http2ConfirmedResponseUnprojectableAtEndStream,
                    model_core::diagnostics::LlmPipelineDiagnosticSeverity::Warning, model_core::diagnostics::LlmPipelineDiagnosticStage::Http2,
                ).with_stream_key(&format!("{}#h2:{}", key.stream_key, stream_id)));
            }
            let ended = stream.end_stream;
            let after = stream.retention_footprint();
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(before.bytes)
                .saturating_add(after.bytes);
            self.retained_ranges = self
                .retained_ranges
                .saturating_sub(before.ranges)
                .saturating_add(after.ranges);
            output.extend(projected);
            if ended {
                self.remove(stream_id);
            }
        }
        output
    }
}

struct RetentionFootprint {
    bytes: usize,
    ranges: usize,
}

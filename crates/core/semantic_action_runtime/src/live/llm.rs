//! Live LLM projection from retained plaintext payload segments.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::payload_projection::llm::{
    LiveLlmProjection, PayloadStreamGroupKey, live_llm_http_response_message_len,
    live_llm_request_message_len, live_llm_request_stream_id_hint,
    project_live_llm_request_message, project_live_llm_response_message,
};

mod call;
mod http;

pub(super) struct LiveLlmProjector {
    config: SemanticRetentionConfig,
    streams: BTreeMap<LiveStreamKey, LiveStreamState>,
    emitted_actions: BTreeMap<EmittedLlmAction, SemanticAction>,
}

impl LiveLlmProjector {
    pub(super) fn new(config: SemanticRetentionConfig) -> Self {
        Self {
            config,
            streams: BTreeMap::new(),
            emitted_actions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EmittedLlmAction {
    trace_id: TraceId,
    action_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LiveStreamKey {
    group: PayloadStreamGroupKey,
    direction: LiveStreamDirection,
}

impl LiveStreamKey {
    fn from_segment(segment: &PayloadSegment) -> Self {
        Self {
            group: PayloadStreamGroupKey::from_segment(segment),
            direction: LiveStreamDirection::from(segment.direction),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum LiveStreamDirection {
    Outbound,
    Inbound,
}

impl From<PayloadDirection> for LiveStreamDirection {
    fn from(direction: PayloadDirection) -> Self {
        match direction {
            PayloadDirection::Outbound => Self::Outbound,
            PayloadDirection::Inbound => Self::Inbound,
        }
    }
}

impl LiveLlmProjector {
    pub(super) fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> Vec<SemanticAction> {
        if !self.config.llm_layer_enabled() {
            return Vec::new();
        }
        if !plaintext_http_candidate(segment) {
            return Vec::new();
        }
        let key = LiveStreamKey::from_segment(segment);
        let actions = self
            .streams
            .entry(key.clone())
            .or_default()
            .observe_segment(&self.config, &key, segment);
        self.changed_actions(actions)
    }

    pub(super) fn observe_http_message(&mut self, action: &SemanticAction) -> Vec<SemanticAction> {
        let emitted_actions = self.emitted_actions.values().collect::<Vec<_>>();
        let Some(failed_call) = http::failed_call_for_http_response(action, &emitted_actions)
        else {
            return Vec::new();
        };
        let key = EmittedLlmAction {
            trace_id: failed_call.trace_id,
            action_id: failed_call.action_id.clone(),
        };
        if self
            .emitted_actions
            .get(&key)
            .is_some_and(|existing| existing == &failed_call)
        {
            return Vec::new();
        }
        self.emitted_actions.insert(key, failed_call.clone());
        vec![failed_call]
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.emitted_actions
            .retain(|action, _| action.trace_id != trace_id);
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> Vec<SemanticAction> {
        let mut finalized = Vec::new();
        for (key, action) in self.emitted_actions.iter_mut() {
            if key.trace_id != trace_id || action.status != SemanticActionStatus::InProgress {
                continue;
            }
            if !matches!(
                action.kind,
                SemanticActionKind::LlmCall
                    | SemanticActionKind::LlmResponse
                    | SemanticActionKind::SseStream
            ) {
                continue;
            }
            action.status = SemanticActionStatus::Error;
            action.completeness = SemanticActionCompleteness::Partial;
            action.end_time = Some(finished_at);
            action.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
            finalized.push(action.clone());
        }
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        finalized
    }

    fn changed_actions(&mut self, actions: Vec<SemanticAction>) -> Vec<SemanticAction> {
        let mut changed = Vec::new();
        for action in actions {
            let key = EmittedLlmAction {
                trace_id: action.trace_id,
                action_id: action.action_id.clone(),
            };
            match self.emitted_actions.get(&key) {
                Some(existing) if existing == &action => continue,
                Some(existing) if suppress_repeated_in_progress_response(existing, &action) => {
                    continue;
                }
                _ => {}
            }
            self.emitted_actions.insert(key, action.clone());
            changed.push(action);
        }
        let emitted_actions = self.emitted_actions.values().collect::<Vec<_>>();
        let pending_requests = self.pending_request_markers();
        let call_candidates =
            call::llm_call_actions_for(&changed, &emitted_actions, &pending_requests);
        for action in call_candidates {
            let key = EmittedLlmAction {
                trace_id: action.trace_id,
                action_id: action.action_id.clone(),
            };
            if self
                .emitted_actions
                .get(&key)
                .is_some_and(|existing| existing == &action)
            {
                continue;
            }
            self.emitted_actions.insert(key, action.clone());
            changed.push(action);
        }
        changed
    }

    fn pending_request_markers(&self) -> Vec<call::PendingLlmRequestMarker> {
        self.streams
            .iter()
            .filter(|(key, _)| key.direction == LiveStreamDirection::Outbound)
            .filter_map(|(key, state)| state.pending_request_marker(&key.group))
            .collect()
    }
}

#[derive(Default)]
struct LiveStreamState {
    buffer: Vec<u8>,
    base_offset: usize,
    segments: VecDeque<LiveSegmentRange>,
    partial_response_emitted: bool,
    pending_raw_chunk_terminator: bool,
    completion_detector: ResponseCompletionDetector,
}

impl LiveStreamState {
    fn observe_segment(
        &mut self,
        config: &SemanticRetentionConfig,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
    ) -> Vec<SemanticAction> {
        self.append_segment(segment);
        match key.direction {
            LiveStreamDirection::Outbound => self.project_outbound_requests(config, &key.group),
            LiveStreamDirection::Inbound => self.project_inbound_responses(config, &key.group),
        }
    }

    fn append_segment(&mut self, segment: &PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(&segment.bytes);
        let end = self.base_offset + self.buffer.len();
        let mut metadata = segment.clone();
        metadata.bytes.clear();
        self.segments.push_back(LiveSegmentRange {
            start,
            end,
            segment: metadata,
        });
        if segment.direction == PayloadDirection::Inbound {
            self.completion_detector.observe(&segment.bytes);
        }
    }

    fn project_outbound_requests(
        &mut self,
        config: &SemanticRetentionConfig,
        key: &PayloadStreamGroupKey,
    ) -> Vec<SemanticAction> {
        let mut actions = Vec::new();
        while let Some(encoded_len) = live_llm_request_message_len(&self.buffer) {
            let message_start = self.base_offset;
            let message_end = message_start + encoded_len;
            let segments = self.segments_for_range(message_start, message_end);
            let Some(projection) = project_live_llm_request_message(
                config,
                key,
                message_start,
                &self.buffer,
                &segments,
            ) else {
                break;
            };
            actions.extend(projection.actions);
            self.evict_encoded_len(encoded_len);
            if self.buffer.is_empty() {
                break;
            }
        }
        actions
    }

    fn project_inbound_responses(
        &mut self,
        config: &SemanticRetentionConfig,
        key: &PayloadStreamGroupKey,
    ) -> Vec<SemanticAction> {
        self.discard_pending_raw_chunk_terminator();
        if self.partial_response_emitted && !self.completion_detector.seen() {
            return Vec::new();
        }

        let mut actions = Vec::new();
        while let Some(projection) = self.project_next_response(config, key) {
            let terminal = projection.terminal;
            let encoded_len = projection.encoded_len;
            actions.extend(projection.actions);
            if terminal {
                self.pending_raw_chunk_terminator = projection.raw_response;
                self.evict_encoded_len(encoded_len);
                self.partial_response_emitted = false;
                self.completion_detector.rebuild(&self.buffer);
                if self.buffer.is_empty() {
                    break;
                }
            } else {
                self.partial_response_emitted = true;
                break;
            }
        }
        actions
    }

    fn project_next_response(
        &self,
        config: &SemanticRetentionConfig,
        key: &PayloadStreamGroupKey,
    ) -> Option<LiveLlmProjection> {
        let encoded_len =
            live_llm_http_response_message_len(&self.buffer).unwrap_or_else(|| self.buffer.len());
        let message_start = self.base_offset;
        let message_end = message_start + encoded_len;
        let segments = self.segments_for_range(message_start, message_end);
        project_live_llm_response_message(config, key, message_start, &self.buffer, &segments)
    }

    fn segments_for_range(&self, start: usize, end: usize) -> Vec<&PayloadSegment> {
        self.segments
            .iter()
            .filter(|range| range.start < end && range.end > start)
            .map(|range| &range.segment)
            .collect()
    }

    fn evict_encoded_len(&mut self, encoded_len: usize) {
        let global_end = self.base_offset + encoded_len;
        self.buffer.drain(..encoded_len);
        self.base_offset = global_end;
        while self
            .segments
            .front()
            .is_some_and(|range| range.end <= self.base_offset)
        {
            self.segments.pop_front();
        }
        if let Some(front) = self.segments.front_mut()
            && front.start < self.base_offset
        {
            front.start = self.base_offset;
        }
    }

    fn discard_pending_raw_chunk_terminator(&mut self) {
        if !self.pending_raw_chunk_terminator {
            return;
        }
        match raw_chunk_terminator_prefix(&self.buffer) {
            RawChunkTerminatorPrefix::None => {
                self.pending_raw_chunk_terminator = false;
            }
            RawChunkTerminatorPrefix::Incomplete => {}
            RawChunkTerminatorPrefix::Prefix(len) => {
                self.evict_encoded_len(len);
            }
            RawChunkTerminatorPrefix::Complete(len) => {
                self.evict_encoded_len(len);
                self.pending_raw_chunk_terminator = false;
            }
        }
    }

    fn pending_request_marker(
        &self,
        key: &PayloadStreamGroupKey,
    ) -> Option<call::PendingLlmRequestMarker> {
        if self.buffer.is_empty() {
            return None;
        }
        let first = self.segments.front()?;
        let http_stream_id = live_llm_request_stream_id_hint(&self.buffer)?;
        Some(call::PendingLlmRequestMarker {
            trace_id: key.trace_id,
            process: key.process.clone(),
            stream_key: key.stream_key.clone(),
            http_stream_id: http_stream_id.map(|id| id.to_string()),
            sequence_start: first.segment.sequence,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RawChunkTerminatorPrefix {
    None,
    Incomplete,
    Prefix(usize),
    Complete(usize),
}

fn raw_chunk_terminator_prefix(bytes: &[u8]) -> RawChunkTerminatorPrefix {
    const CRLF: &[u8] = b"\r\n";
    const ZERO_CHUNK: &[u8] = b"0\r\n\r\n";
    const CRLF_ZERO_CHUNK: &[u8] = b"\r\n0\r\n\r\n";

    if bytes.is_empty() {
        return RawChunkTerminatorPrefix::Incomplete;
    }
    if bytes.starts_with(CRLF_ZERO_CHUNK) {
        return RawChunkTerminatorPrefix::Complete(CRLF_ZERO_CHUNK.len());
    }
    if bytes.starts_with(ZERO_CHUNK) {
        return RawChunkTerminatorPrefix::Complete(ZERO_CHUNK.len());
    }
    if CRLF_ZERO_CHUNK.starts_with(bytes) || ZERO_CHUNK.starts_with(bytes) {
        return RawChunkTerminatorPrefix::Incomplete;
    }
    if bytes.starts_with(CRLF) {
        return RawChunkTerminatorPrefix::Prefix(CRLF.len());
    }
    RawChunkTerminatorPrefix::None
}

struct LiveSegmentRange {
    start: usize,
    end: usize,
    segment: PayloadSegment,
}

#[derive(Default)]
struct ResponseCompletionDetector {
    tail: Vec<u8>,
    seen: bool,
}

impl ResponseCompletionDetector {
    fn observe(&mut self, bytes: &[u8]) {
        if self.seen {
            return;
        }
        let mut window = Vec::with_capacity(self.tail.len() + bytes.len());
        window.extend_from_slice(&self.tail);
        window.extend_from_slice(bytes);
        self.seen = response_completion_marker_seen(&window);
        self.tail = response_completion_tail(&window);
    }

    fn rebuild(&mut self, bytes: &[u8]) {
        self.tail.clear();
        self.seen = false;
        self.observe(bytes);
    }

    fn seen(&self) -> bool {
        self.seen
    }
}

fn response_completion_marker_seen(bytes: &[u8]) -> bool {
    contains_subslice(bytes, b"[DONE]") || contains_subslice(bytes, b"message_stop")
}

fn response_completion_tail(bytes: &[u8]) -> Vec<u8> {
    let marker_window = b"message_stop".len().max(b"[DONE]".len());
    let tail_len = marker_window.saturating_sub(1).min(bytes.len());
    bytes[bytes.len() - tail_len..].to_vec()
}

fn contains_subslice(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn plaintext_http_candidate(segment: &PayloadSegment) -> bool {
    matches!(
        segment.source_boundary,
        PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
    ) && segment.content_state == PayloadContentState::Plaintext
}

fn suppress_repeated_in_progress_response(
    existing: &SemanticAction,
    candidate: &SemanticAction,
) -> bool {
    existing.kind == SemanticActionKind::LlmResponse
        && candidate.kind == SemanticActionKind::LlmResponse
        && existing.status == SemanticActionStatus::InProgress
        && candidate.status == SemanticActionStatus::InProgress
}

//! Live LLM projection from retained plaintext payload segments.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::payload_projection::llm::{
    LiveLlmProjection, PayloadStreamGroupKey, live_llm_http_response_message_len,
    live_llm_request_message_len, live_llm_request_prefix_skip_len,
    live_llm_request_stream_id_hint, project_live_llm_request_message,
    project_live_llm_response_message,
};

use super::actions::action_for_live_state;

mod call;
mod http;

pub(super) struct LiveLlmProjector {
    config: SemanticRetentionConfig,
    streams: BTreeMap<LiveStreamKey, LiveStreamState>,
    open_requests: BTreeMap<LlmStreamKey, VecDeque<OpenLlmRequest>>,
    pending_responses: BTreeMap<LlmStreamKey, VecDeque<SemanticAction>>,
    open_calls_by_request: BTreeMap<(TraceId, String), SemanticAction>,
    open_action_versions: BTreeMap<(TraceId, String), SemanticAction>,
}

impl LiveLlmProjector {
    pub(super) fn new(config: SemanticRetentionConfig) -> Self {
        Self {
            config,
            streams: BTreeMap::new(),
            open_requests: BTreeMap::new(),
            pending_responses: BTreeMap::new(),
            open_calls_by_request: BTreeMap::new(),
            open_action_versions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LlmStreamKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
    http_stream_id: Option<String>,
}

#[derive(Clone, Debug)]
struct OpenLlmRequest {
    action: SemanticAction,
    sequence_start: u64,
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

impl LlmStreamKey {
    fn from_llm_request(action: &SemanticAction) -> Option<Self> {
        Self::from_llm_action(action, attrs::http_request::STREAM_ID)
    }

    fn from_llm_response(action: &SemanticAction) -> Option<Self> {
        Self::from_llm_action(action, attrs::http_response::STREAM_ID)
    }

    fn from_llm_action(action: &SemanticAction, http_stream_attr: &'static str) -> Option<Self> {
        Some(Self {
            trace_id: action.trace_id,
            process: action.process.clone(),
            stream_key: action.attributes.get(attrs::payload::STREAM_KEY)?.clone(),
            http_stream_id: action.attributes.get(http_stream_attr).cloned(),
        })
    }

    fn from_http_response_candidates(action: &SemanticAction) -> Vec<Self> {
        let Some(stream_key) = action.attributes.get("stream_key") else {
            return Vec::new();
        };
        let http_stream_id = action.attributes.get("stream_id").cloned();
        let exact = Self {
            trace_id: action.trace_id,
            process: action.process.clone(),
            stream_key: stream_key.clone(),
            http_stream_id: http_stream_id.clone(),
        };
        if http_stream_id.is_some() {
            vec![
                exact,
                Self {
                    trace_id: action.trace_id,
                    process: action.process.clone(),
                    stream_key: stream_key.clone(),
                    http_stream_id: None,
                },
            ]
        } else {
            vec![exact]
        }
    }

    fn matches_pending_request(&self, pending: &call::PendingLlmRequestMarker) -> bool {
        self.trace_id == pending.trace_id
            && self.process == pending.process
            && self.stream_key == pending.stream_key
            && self.http_stream_id == pending.http_stream_id
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
        if !http::error_response(action) {
            return Vec::new();
        }
        let Some((request, call)) = self.take_open_request_for_http_response(action) else {
            return Vec::new();
        };
        let Some(failed_call) = http::failed_call_for_open_request(action, &request, &call) else {
            self.restore_open_request(request, call);
            return Vec::new();
        };
        self.record_projected_action(&failed_call)
            .then_some(failed_call)
            .into_iter()
            .collect()
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.open_requests.retain(|key, _| key.trace_id != trace_id);
        self.pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.open_calls_by_request
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.open_action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> Vec<SemanticAction> {
        let mut finalized = Vec::new();
        for ((candidate, _), action) in self.open_action_versions.iter_mut() {
            if *candidate != trace_id || action.status != SemanticActionStatus::InProgress {
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
        self.open_requests.retain(|key, _| key.trace_id != trace_id);
        self.pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.open_calls_by_request
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.open_action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
        finalized
    }

    fn changed_actions(&mut self, actions: Vec<SemanticAction>) -> Vec<SemanticAction> {
        let mut changed = Vec::new();
        for action in actions {
            let state_action = action_for_live_state(&action);
            if self.record_projected_action(&state_action) {
                changed.push(action.clone());
            }
            match action.kind {
                SemanticActionKind::LlmRequest => {
                    self.remember_open_request(state_action.clone());
                    if let Some(response) = self.take_pending_response_for_request(&state_action) {
                        let call =
                            call::llm_call_from_request_response(&state_action, Some(&response));
                        self.open_calls_by_request
                            .remove(&(state_action.trace_id, state_action.action_id.clone()));
                        self.remove_open_request(&state_action);
                        if self.record_projected_action(&call) {
                            changed.push(call);
                        }
                    } else {
                        let call = call::llm_call_from_request_response(&state_action, None);
                        self.open_calls_by_request.insert(
                            (state_action.trace_id, state_action.action_id.clone()),
                            call.clone(),
                        );
                        if self.record_projected_action(&call) {
                            changed.push(call);
                        }
                    }
                }
                SemanticActionKind::LlmResponse => {
                    let request = if state_action.status == SemanticActionStatus::InProgress {
                        self.open_request_for_response(&state_action)
                    } else {
                        self.take_open_request_for_response(&state_action)
                    };
                    if let Some(request) = request {
                        let call =
                            call::llm_call_from_request_response(&request, Some(&state_action));
                        if state_action.status == SemanticActionStatus::InProgress {
                            self.open_calls_by_request.insert(
                                (request.trace_id, request.action_id.clone()),
                                call.clone(),
                            );
                        } else {
                            self.open_calls_by_request
                                .remove(&(request.trace_id, request.action_id.clone()));
                        }
                        if self.record_projected_action(&call) {
                            changed.push(call);
                        }
                    } else {
                        self.remember_pending_response(state_action.clone());
                    }
                }
                _ => {}
            }
        }
        changed
    }

    fn record_projected_action(&mut self, action: &SemanticAction) -> bool {
        let key = (action.trace_id, action.action_id.clone());
        match self.open_action_versions.get(&key) {
            Some(existing) if existing == action => return false,
            Some(existing) if suppress_repeated_in_progress_response(existing, action) => {
                return false;
            }
            _ => {}
        }
        if action.status == SemanticActionStatus::InProgress {
            self.open_action_versions.insert(key, action.clone());
        } else {
            self.open_action_versions.remove(&key);
        }
        true
    }

    fn remember_open_request(&mut self, request: SemanticAction) {
        let Some(stream_key) = LlmStreamKey::from_llm_request(&request) else {
            return;
        };
        let Some(sequence_start) = call::payload_sequence_start(&request) else {
            return;
        };
        let requests = self.open_requests.entry(stream_key).or_default();
        if let Some(existing) = requests
            .iter_mut()
            .find(|candidate| candidate.action.action_id == request.action_id)
        {
            existing.action = request;
            existing.sequence_start = sequence_start;
        } else {
            requests.push_back(OpenLlmRequest {
                action: request,
                sequence_start,
            });
        }
    }

    fn restore_open_request(&mut self, request: SemanticAction, call: SemanticAction) {
        self.remember_open_request(request.clone());
        self.open_calls_by_request
            .insert((request.trace_id, request.action_id.clone()), call);
    }

    fn remove_open_request(&mut self, request: &SemanticAction) {
        let Some(stream_key) = LlmStreamKey::from_llm_request(request) else {
            return;
        };
        let Some(requests) = self.open_requests.get_mut(&stream_key) else {
            return;
        };
        requests.retain(|candidate| candidate.action.action_id != request.action_id);
        if requests.is_empty() {
            self.open_requests.remove(&stream_key);
        }
    }

    fn remember_pending_response(&mut self, response: SemanticAction) {
        let Some(stream_key) = LlmStreamKey::from_llm_response(&response) else {
            return;
        };
        let responses = self.pending_responses.entry(stream_key).or_default();
        if responses
            .iter()
            .any(|candidate| candidate.action_id == response.action_id)
        {
            return;
        }
        responses.push_back(response);
    }

    fn take_pending_response_for_request(
        &mut self,
        request: &SemanticAction,
    ) -> Option<SemanticAction> {
        let stream_key = LlmStreamKey::from_llm_request(request)?;
        let request_sequence = call::payload_sequence_start(request)?;
        let responses = self.pending_responses.get_mut(&stream_key)?;
        let selected = responses
            .iter()
            .enumerate()
            .filter(|(_, response)| {
                call::payload_sequence_start(response)
                    .is_some_and(|response_sequence| request_sequence <= response_sequence)
            })
            .min_by_key(|(_, response)| {
                (
                    call::payload_sequence_start(response).unwrap_or_default(),
                    response.action_id.clone(),
                )
            })
            .map(|(index, _)| index)?;
        let response = responses.remove(selected)?;
        if responses.is_empty() {
            self.pending_responses.remove(&stream_key);
        }
        Some(response)
    }

    fn take_open_request_for_response(
        &mut self,
        response: &SemanticAction,
    ) -> Option<SemanticAction> {
        let stream_key = LlmStreamKey::from_llm_response(response)?;
        let response_sequence = call::payload_sequence_start(response)?;
        self.take_open_request_before(&stream_key, response_sequence)
    }

    fn open_request_for_response(&self, response: &SemanticAction) -> Option<SemanticAction> {
        let stream_key = LlmStreamKey::from_llm_response(response)?;
        let response_sequence = call::payload_sequence_start(response)?;
        self.open_request_before(&stream_key, response_sequence)
    }

    fn take_open_request_for_http_response(
        &mut self,
        http_response: &SemanticAction,
    ) -> Option<(SemanticAction, SemanticAction)> {
        let response_sequence = http_payload_sequence(http_response)?;
        for stream_key in LlmStreamKey::from_http_response_candidates(http_response) {
            let Some(request) = self.take_open_request_before(&stream_key, response_sequence)
            else {
                continue;
            };
            let Some(call) = self
                .open_calls_by_request
                .remove(&(request.trace_id, request.action_id.clone()))
            else {
                continue;
            };
            return Some((request, call));
        }
        None
    }

    fn take_open_request_before(
        &mut self,
        stream_key: &LlmStreamKey,
        response_sequence: u64,
    ) -> Option<SemanticAction> {
        let selected = self.select_open_request_before(stream_key, response_sequence)?;
        let requests = self.open_requests.get_mut(stream_key)?;
        let request = requests.remove(selected)?.action;
        if requests.is_empty() {
            self.open_requests.remove(stream_key);
        }
        Some(request)
    }

    fn open_request_before(
        &self,
        stream_key: &LlmStreamKey,
        response_sequence: u64,
    ) -> Option<SemanticAction> {
        let selected = self.select_open_request_before(stream_key, response_sequence)?;
        self.open_requests
            .get(stream_key)?
            .get(selected)
            .map(|request| request.action.clone())
    }

    fn select_open_request_before(
        &self,
        stream_key: &LlmStreamKey,
        response_sequence: u64,
    ) -> Option<usize> {
        let requests = self.open_requests.get(stream_key)?;
        let selected = requests
            .iter()
            .enumerate()
            .filter(|(_, request)| request.sequence_start <= response_sequence)
            .max_by_key(|(_, request)| (request.sequence_start, request.action.action_id.clone()))
            .map(|(index, _)| index)?;
        let request_sequence = requests.get(selected)?.sequence_start;
        if self.pending_request_between(stream_key, request_sequence, response_sequence) {
            return None;
        }
        Some(selected)
    }

    fn pending_request_between(
        &self,
        stream_key: &LlmStreamKey,
        request_sequence: u64,
        response_sequence: u64,
    ) -> bool {
        self.pending_request_markers()
            .iter()
            .filter(|pending| stream_key.matches_pending_request(pending))
            .any(|pending| {
                request_sequence < pending.sequence_start
                    && pending.sequence_start <= response_sequence
            })
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
        loop {
            if let Some(skip_len) = live_llm_request_prefix_skip_len(&self.buffer) {
                self.evict_encoded_len(skip_len);
                if self.buffer.is_empty() {
                    break;
                }
                continue;
            }
            let Some(encoded_len) = live_llm_request_message_len(&self.buffer) else {
                break;
            };
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

fn http_payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get("payload_sequence")?.parse().ok()
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

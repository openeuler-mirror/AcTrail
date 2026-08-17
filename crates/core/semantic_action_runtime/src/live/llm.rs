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
    LlmRequestContentWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionStatus, SemanticEvidenceKind, attr_keys as attrs,
};

use crate::payload_projection::http::{
    HTTP2_CONNECTION_PREFACE, HTTP2_CONTINUATION_FRAME_TYPE, HTTP2_DATA_FRAME_TYPE,
    HTTP2_FLAG_END_STREAM, HTTP2_HEADERS_FRAME_TYPE, decode_http2_frame, http2_data_payload,
};
use crate::payload_projection::llm::{
    InFlightResponse, IncrementalSseCache, LiveLlmProjection, LiveLlmResponseMessage,
    LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry, PayloadStreamGroupKey,
    live_llm_request_message_len, live_llm_request_prefix_skip_len,
    live_llm_request_stream_id_hint, project_http2_stream_request, project_http2_stream_response,
    project_live_llm_request_message, project_live_llm_response_message, semantic_payload_draft,
};

use super::actions::action_for_live_state;

mod call;
mod http;
mod websocket;

pub(super) struct LiveLlmProjector {
    config: SemanticRetentionConfig,
    codecs: LlmCodecRegistry,
    streams: BTreeMap<LiveStreamKey, LiveStreamState>,
    open_requests: BTreeMap<LlmStreamKey, VecDeque<OpenLlmRequest>>,
    pending_responses: BTreeMap<LlmStreamKey, VecDeque<SemanticAction>>,
    open_action_versions: BTreeMap<(TraceId, String), SemanticAction>,
    websocket: websocket::WebSocketLlmAdapter,
}

#[derive(Default)]
pub(super) struct LiveLlmOutput {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) llm_request_contents: Vec<LlmRequestContentWrite>,
    pub(super) payload_segments: Vec<PayloadSegment>,
}

impl LiveLlmOutput {
    fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.payload_segments.extend(other.payload_segments);
    }
}

impl LiveLlmProjector {
    pub(super) fn new(config: SemanticRetentionConfig) -> Self {
        Self {
            config,
            codecs: LlmCodecRegistry::default(),
            streams: BTreeMap::new(),
            open_requests: BTreeMap::new(),
            pending_responses: BTreeMap::new(),
            open_action_versions: BTreeMap::new(),
            websocket: websocket::WebSocketLlmAdapter::default(),
        }
    }
}

impl LiveLlmProjector {
    pub(super) fn register_codec(
        &mut self,
        plugin: std::sync::Arc<dyn LlmCodecPlugin>,
    ) -> Result<(), String> {
        self.codecs.register(plugin)
    }

    pub(super) fn unregister_codec(&mut self, instance_id: &str) -> bool {
        self.codecs.unregister(instance_id)
    }

    pub(super) fn codec_statuses(&self) -> Vec<LlmCodecPluginStatus> {
        self.codecs.statuses()
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
    start_time: SystemTime,
    sequence_start: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LlmActionOrder {
    start_time: SystemTime,
    sequence_start: u64,
}

impl LlmActionOrder {
    fn from_action(action: &SemanticAction) -> Option<Self> {
        Some(Self {
            start_time: action.start_time,
            sequence_start: call::payload_sequence_start(action)?,
        })
    }
}

impl OpenLlmRequest {
    fn order(&self) -> LlmActionOrder {
        LlmActionOrder {
            start_time: self.start_time,
            sequence_start: self.sequence_start,
        }
    }
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
    pub(super) fn observe_payload_segment(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        if !self.config.llm_layer_enabled() {
            return LiveLlmOutput::default();
        }
        if !plaintext_http_candidate(segment) {
            return LiveLlmOutput::default();
        }
        let synthetic = self.websocket.observe(segment);
        let mut changed = self.observe_http_payload(segment);
        for candidate in &synthetic {
            changed.extend(self.observe_http_payload(candidate));
        }
        changed
    }

    pub(super) fn observe_http_message(&mut self, action: &SemanticAction) -> Vec<SemanticAction> {
        if !http::terminal_failure_response(action) {
            return Vec::new();
        }
        let Some(request) = self.take_open_request_for_http_response(action) else {
            return Vec::new();
        };
        let call = call::llm_call_from_request_response(&request, None);
        let Some(failed_response) = http::failed_response_for_open_request(action, &request, &call)
        else {
            self.restore_open_request(request);
            return Vec::new();
        };
        let failed_call = call::llm_call_from_request_response(&request, Some(&failed_response));
        let mut actions = Vec::new();
        if self.record_projected_action(&failed_response) {
            actions.push(failed_response);
        }
        if self.record_projected_action(&failed_call) {
            actions.push(failed_call);
        }
        actions
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.websocket.forget_trace(trace_id);
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.open_requests.retain(|key, _| key.trace_id != trace_id);
        self.pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.open_action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> (Vec<SemanticAction>, Vec<PayloadSegment>) {
        self.websocket.forget_trace(trace_id);
        let mut finalized = Vec::new();
        let mut payload_segments = Vec::new();
        let keys = self
            .streams
            .keys()
            .filter(|key| {
                key.group.trace_id == trace_id && key.direction == LiveStreamDirection::Inbound
            })
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            let Some(mut state) = self.streams.remove(&key) else {
                continue;
            };
            let Some(in_flight) = state.take_in_flight_response() else {
                continue;
            };
            let Some((mut actions, drafts)) = state.materialize_in_flight(
                &self.config,
                &self.codecs,
                &key.group,
                in_flight.message_start,
            ) else {
                continue;
            };
            payload_segments.extend(drafts);
            for action in &mut actions {
                if action.kind != SemanticActionKind::LlmResponse {
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
                if let Some(request) = self.open_request_for_response(action) {
                    let mut call = call::llm_call_from_request_response(&request, Some(action));
                    call.status = SemanticActionStatus::Error;
                    call.completeness = SemanticActionCompleteness::Partial;
                    call.end_time = Some(finished_at);
                    finalized.push(call);
                }
            }
        }
        for request in self.open_requests_for_trace(trace_id) {
            let mut call = call::llm_call_from_request_response(&request, None);
            call.status = SemanticActionStatus::Error;
            call.completeness = SemanticActionCompleteness::Partial;
            call.end_time = Some(finished_at);
            call.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                "true".to_string(),
            );
            finalized.push(call);
        }
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.open_requests.retain(|key, _| key.trace_id != trace_id);
        self.pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.open_action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
        (finalized, payload_segments)
    }

    fn open_requests_for_trace(&self, trace_id: TraceId) -> Vec<SemanticAction> {
        self.open_requests
            .iter()
            .filter(|(key, _)| key.trace_id == trace_id)
            .flat_map(|(_, requests)| requests.iter().map(|request| request.action.clone()))
            .collect()
    }

    fn observe_http_payload(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        let key = LiveStreamKey::from_segment(segment);
        let output = self
            .streams
            .entry(key.clone())
            .or_default()
            .observe_segment(&self.config, &self.codecs, &key, segment);
        self.changed_actions(output)
    }

    fn changed_actions(&mut self, output: LiveLlmOutput) -> LiveLlmOutput {
        let mut changed = LiveLlmOutput::default();
        let mut request_contents = output
            .llm_request_contents
            .into_iter()
            .map(|content| (content.manifest.action_id.clone(), content))
            .collect::<BTreeMap<_, _>>();
        for mut action in output.actions {
            if !self.config.l4_payload.enabled {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            let state_action = action_for_live_state(&action);
            let action_changed = self.record_projected_action(&state_action);
            if action_changed {
                if let Some(content) = request_contents.remove(&action.action_id) {
                    changed.llm_request_contents.push(content);
                }
                changed.actions.push(action);
            }
            match state_action.kind {
                SemanticActionKind::LlmRequest => {
                    self.remember_open_request(state_action.clone());
                    if let Some(response) = self.take_pending_response_for_request(&state_action) {
                        let call =
                            call::llm_call_from_request_response(&state_action, Some(&response));
                        self.remove_open_request(&state_action);
                        self.record_derived_call(call, &mut changed.actions);
                    }
                }
                SemanticActionKind::LlmResponse => {
                    if let Some(request) = self.take_open_request_for_response(&state_action) {
                        let call =
                            call::llm_call_from_request_response(&request, Some(&state_action));
                        self.record_derived_call(call, &mut changed.actions);
                    } else {
                        self.remember_pending_response(state_action.clone());
                    }
                }
                _ => {}
            }
        }
        changed
    }

    fn record_derived_call(&mut self, call: SemanticAction, changed: &mut Vec<SemanticAction>) {
        if self.record_projected_action(&call) {
            changed.push(call);
        }
    }

    fn record_projected_action(&mut self, action: &SemanticAction) -> bool {
        let key = (action.trace_id, action.action_id.clone());
        if self.open_action_versions.get(&key) == Some(action) {
            return false;
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
            existing.start_time = request.start_time;
            existing.action = request;
            existing.sequence_start = sequence_start;
        } else {
            requests.push_back(OpenLlmRequest {
                start_time: request.start_time,
                action: request,
                sequence_start,
            });
        }
    }

    fn restore_open_request(&mut self, request: SemanticAction) {
        self.remember_open_request(request);
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
        let request_order = LlmActionOrder::from_action(request)?;
        let responses = self.pending_responses.get_mut(&stream_key)?;
        let selected = responses
            .iter()
            .enumerate()
            .filter(|(_, response)| {
                LlmActionOrder::from_action(response)
                    .is_some_and(|response_order| request_order <= response_order)
            })
            .min_by_key(|(_, response)| {
                (
                    LlmActionOrder::from_action(response).unwrap_or(LlmActionOrder {
                        start_time: response.start_time,
                        sequence_start: u64::MAX,
                    }),
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
        let response_order = LlmActionOrder::from_action(response)?;
        self.take_open_request_before(&stream_key, response_order)
    }

    fn open_request_for_response(&self, response: &SemanticAction) -> Option<SemanticAction> {
        let stream_key = LlmStreamKey::from_llm_response(response)?;
        let response_order = LlmActionOrder::from_action(response)?;
        self.open_request_before(&stream_key, response_order)
    }

    fn take_open_request_for_http_response(
        &mut self,
        http_response: &SemanticAction,
    ) -> Option<SemanticAction> {
        let response_order = LlmActionOrder {
            start_time: http_response.start_time,
            sequence_start: http_payload_sequence(http_response)?,
        };
        for stream_key in LlmStreamKey::from_http_response_candidates(http_response) {
            let Some(request) = self.take_open_request_before(&stream_key, response_order) else {
                continue;
            };
            return Some(request);
        }
        None
    }

    fn take_open_request_before(
        &mut self,
        stream_key: &LlmStreamKey,
        response_order: LlmActionOrder,
    ) -> Option<SemanticAction> {
        let selected = self.select_open_request_before(stream_key, response_order)?;
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
        response_order: LlmActionOrder,
    ) -> Option<SemanticAction> {
        let selected = self.select_open_request_before(stream_key, response_order)?;
        self.open_requests
            .get(stream_key)?
            .get(selected)
            .map(|request| request.action.clone())
    }

    fn select_open_request_before(
        &self,
        stream_key: &LlmStreamKey,
        response_order: LlmActionOrder,
    ) -> Option<usize> {
        let requests = self.open_requests.get(stream_key)?;
        let selected = requests
            .iter()
            .enumerate()
            .filter(|(_, request)| request.order() <= response_order)
            .max_by_key(|(_, request)| (request.order(), request.action.action_id.clone()))
            .map(|(index, _)| index)?;
        let request_order = requests.get(selected)?.order();
        if self.pending_request_between(stream_key, request_order, response_order) {
            return None;
        }
        Some(selected)
    }

    fn pending_request_between(
        &self,
        stream_key: &LlmStreamKey,
        request_order: LlmActionOrder,
        response_order: LlmActionOrder,
    ) -> bool {
        self.pending_request_markers()
            .iter()
            .filter(|pending| stream_key.matches_pending_request(pending))
            .any(|pending| {
                let pending_order = LlmActionOrder {
                    start_time: pending.start_time,
                    sequence_start: pending.sequence_start,
                };
                request_order < pending_order && pending_order <= response_order
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

/// One sequential plaintext byte stream to assemble and project: a whole
/// HTTP/1 (or raw) connection body, or one de-multiplexed HTTP/2 stream's
/// plaintext (its DATA-frame payloads).
#[derive(Default)]
struct PlainStreamAssembly {
    buffer: Vec<u8>,
    base_offset: usize,
    segments: VecDeque<LiveSegmentRange>,
    pending_raw_chunk_terminator: bool,
    completion_detector: ResponseCompletionDetector,
    sse_parse_cache: Option<IncrementalSseCache>,
    in_flight_response: Option<InFlightResponse>,
}

impl PlainStreamAssembly {
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

    /// Append de-framed plaintext (e.g. one HTTP/2 DATA payload) attributed to
    /// a captured segment.
    fn append_plaintext(&mut self, bytes: &[u8], segment: PayloadSegment) {
        let start = self.base_offset + self.buffer.len();
        self.buffer.extend_from_slice(bytes);
        let end = self.base_offset + self.buffer.len();
        let mut metadata = segment;
        metadata.bytes.clear();
        self.segments.push_back(LiveSegmentRange {
            start,
            end,
            segment: metadata,
        });
        self.completion_detector.observe(bytes);
    }

    fn project_outbound_requests(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
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
                codecs,
                key,
                message_start,
                &self.buffer,
                &segments,
            ) else {
                break;
            };
            output.actions.extend(projection.actions);
            output
                .llm_request_contents
                .extend(projection.llm_request_contents);
            output.payload_segments.extend(projection.payload_segments);
            self.evict_encoded_len(encoded_len);
            if self.buffer.is_empty() {
                break;
            }
        }
        output
    }

    fn project_inbound_responses(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        self.discard_pending_raw_chunk_terminator();

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
            output.payload_segments.extend(projection.payload_segments);
            if terminal {
                self.pending_raw_chunk_terminator = projection.raw_response;
                self.evict_encoded_len(encoded_len);
                self.sse_parse_cache = None;
                self.completion_detector.rebuild(&self.buffer);
                if self.buffer.is_empty() {
                    break;
                }
            } else {
                break;
            }
        }
        output
    }

    fn project_next_response(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
    ) -> Option<LiveLlmProjection> {
        let mut sse_parse_cache = self.sse_parse_cache.take();
        let message = LiveLlmResponseMessage::parse(&self.buffer);
        let encoded_len = message.encoded_len();
        let message_start = self.base_offset;
        let message_end = message_start + encoded_len;
        let segments = self.segments_for_range(message_start, message_end);
        let projection = project_live_llm_response_message(
            config,
            codecs,
            key,
            message_start,
            &self.buffer,
            message,
            &segments,
            &mut sse_parse_cache,
            false,
        );
        self.sse_parse_cache = sse_parse_cache;
        projection
    }

    fn materialize_in_flight(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        message_start: usize,
    ) -> Option<(Vec<SemanticAction>, Vec<PayloadSegment>)> {
        let mut sse_parse_cache = self.sse_parse_cache.take();
        let message = LiveLlmResponseMessage::parse(&self.buffer);
        let encoded_len = message.encoded_len();
        let message_end = message_start.checked_add(encoded_len)?;
        let (first, assembled_bytes, projection) = {
            let segments = self.segments_for_range(message_start, message_end);
            let first = (**segments.first()?).clone();
            let assembled_bytes = self.buffer.get(..encoded_len)?.to_vec();
            let projection = project_live_llm_response_message(
                config,
                codecs,
                key,
                message_start,
                &self.buffer,
                message,
                &segments,
                &mut sse_parse_cache,
                true,
            );
            (first, assembled_bytes, projection)
        };
        self.sse_parse_cache = sse_parse_cache;
        let payload_segments =
            if config.l4_payload.enabled || !config.l0_llm_call.retain_assembled_payload() {
                Vec::new()
            } else {
                vec![semantic_payload_draft(&first, &assembled_bytes)]
            };
        Some((projection?.actions, payload_segments))
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
            start_time: first.segment.observed_at,
            sequence_start: first.segment.sequence,
        })
    }
}

/// One HTTP/2 stream's de-multiplexed plaintext plus its end-of-stream flag.
#[derive(Default)]
struct Http2StreamAssembly {
    plain: PlainStreamAssembly,
    end_stream: bool,
}

impl Http2StreamAssembly {
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
            &segments,
        ) else {
            return output;
        };
        output.actions.extend(projection.actions);
        output
            .llm_request_contents
            .extend(projection.llm_request_contents);
        self.plain.evict_encoded_len(projection.encoded_len);
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
        let segments = self.plain.segments_for_range(message_start, message_end);
        let Some(projection) = project_http2_stream_response(
            config,
            codecs,
            key,
            stream_id,
            message_start,
            &self.plain.buffer,
            &segments,
            &mut sse_parse_cache,
            self.end_stream,
        ) else {
            self.plain.sse_parse_cache = sse_parse_cache;
            return output;
        };
        self.plain.sse_parse_cache = sse_parse_cache;
        output.actions.extend(projection.actions);
        if projection.terminal {
            self.plain.evict_encoded_len(projection.encoded_len);
        }
        output
    }
}

/// A whole HTTP/2 connection in one direction: the raw frame byte stream,
/// decomposed into per-stream plaintext assemblies.
struct Http2ConnectionAssembly {
    frame_buffer: Vec<u8>,
    frame_base_offset: usize,
    frame_segments: VecDeque<LiveSegmentRange>,
    streams: BTreeMap<u32, Http2StreamAssembly>,
}

impl Default for Http2ConnectionAssembly {
    fn default() -> Self {
        Self {
            frame_buffer: Vec::new(),
            frame_base_offset: 0,
            frame_segments: VecDeque::new(),
            streams: BTreeMap::new(),
        }
    }
}

impl Http2ConnectionAssembly {
    fn append_segment(&mut self, segment: &PayloadSegment) {
        let start = self.frame_base_offset + self.frame_buffer.len();
        self.frame_buffer.extend_from_slice(&segment.bytes);
        let end = self.frame_base_offset + self.frame_buffer.len();
        let mut metadata = segment.clone();
        metadata.bytes.clear();
        self.frame_segments.push_back(LiveSegmentRange {
            start,
            end,
            segment: metadata,
        });
        self.parse_frames();
    }

    fn parse_frames(&mut self) {
        let mut cursor = 0;
        if self.frame_buffer.starts_with(HTTP2_CONNECTION_PREFACE) {
            cursor = HTTP2_CONNECTION_PREFACE.len();
        }
        loop {
            // Copy the frame's info out so we can mutate self while iterating.
            let (frame_type, flags, stream_id, payload, encoded_len) = {
                let Some(frame) = decode_http2_frame(&self.frame_buffer[cursor..]) else {
                    break;
                };
                (
                    frame.frame_type,
                    frame.flags,
                    frame.stream_id,
                    frame.payload.to_vec(),
                    frame.encoded_len,
                )
            };
            let frame_start = self.frame_base_offset + cursor;
            match frame_type {
                HTTP2_DATA_FRAME_TYPE => {
                    if let Some(data) = http2_data_payload(flags, &payload) {
                        self.route_stream_data(stream_id, frame_start, data);
                    }
                    if flags & HTTP2_FLAG_END_STREAM != 0 {
                        self.mark_end_stream(stream_id);
                    }
                }
                HTTP2_HEADERS_FRAME_TYPE | HTTP2_CONTINUATION_FRAME_TYPE => {
                    if flags & HTTP2_FLAG_END_STREAM != 0 {
                        self.mark_end_stream(stream_id);
                    }
                }
                _ => {}
            }
            cursor += encoded_len;
        }
        if cursor > 0 {
            self.evict_frames(cursor);
        }
    }

    fn route_stream_data(&mut self, stream_id: u32, frame_start: usize, data: &[u8]) {
        let Some(segment) = self.segment_metadata_at(frame_start).cloned() else {
            return;
        };
        self.streams
            .entry(stream_id)
            .or_default()
            .plain
            .append_plaintext(data, segment);
    }

    fn segment_metadata_at(&self, global_offset: usize) -> Option<&PayloadSegment> {
        self.frame_segments
            .iter()
            .find(|range| range.start <= global_offset && global_offset < range.end)
            .map(|range| &range.segment)
    }

    fn mark_end_stream(&mut self, stream_id: u32) {
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.end_stream = true;
        }
    }

    fn evict_frames(&mut self, consumed: usize) {
        let global_end = self.frame_base_offset + consumed;
        self.frame_buffer.drain(..consumed);
        self.frame_base_offset = global_end;
        while self
            .frame_segments
            .front()
            .is_some_and(|range| range.end <= self.frame_base_offset)
        {
            self.frame_segments.pop_front();
        }
        if let Some(front) = self.frame_segments.front_mut()
            && front.start < self.frame_base_offset
        {
            front.start = self.frame_base_offset;
        }
    }

    fn project(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        direction: LiveStreamDirection,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let mut done = Vec::new();
        for (stream_id, stream) in self.streams.iter_mut() {
            let projected = match direction {
                LiveStreamDirection::Outbound => {
                    stream.project_request(config, codecs, key, *stream_id)
                }
                LiveStreamDirection::Inbound => {
                    stream.project_response(config, codecs, key, *stream_id)
                }
            };
            output.extend(projected);
            if stream.plain.buffer.is_empty() || stream.end_stream {
                done.push(*stream_id);
            }
        }
        for stream_id in done {
            self.streams.remove(&stream_id);
        }
        output
    }

    fn pending_request_marker(
        &self,
        key: &PayloadStreamGroupKey,
    ) -> Option<call::PendingLlmRequestMarker> {
        for (stream_id, stream) in &self.streams {
            if stream.end_stream {
                continue;
            }
            let first = stream.plain.segments.front()?;
            return Some(call::PendingLlmRequestMarker {
                trace_id: key.trace_id,
                process: key.process.clone(),
                stream_key: key.stream_key.clone(),
                http_stream_id: Some(stream_id.to_string()),
                start_time: first.segment.observed_at,
                sequence_start: first.segment.sequence,
            });
        }
        None
    }
}

/// The byte-stream assembly for one (stream_key, direction): either a plain
/// sequential stream (HTTP/1, raw) or a de-multiplexed HTTP/2 connection.
enum StreamBody {
    Plain(PlainStreamAssembly),
    Http2(Http2ConnectionAssembly),
}

struct LiveStreamState {
    body: StreamBody,
}

impl Default for LiveStreamState {
    fn default() -> Self {
        Self {
            body: StreamBody::Plain(PlainStreamAssembly::default()),
        }
    }
}

impl LiveStreamState {
    fn observe_segment(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
    ) -> LiveLlmOutput {
        match &mut self.body {
            StreamBody::Plain(plain) => {
                plain.append_segment(segment);
                if looks_like_http2(&plain.buffer) {
                    self.activate_http2();
                    match &mut self.body {
                        StreamBody::Http2(http2) => {
                            http2.project(config, codecs, &key.group, key.direction)
                        }
                        StreamBody::Plain(_) => unreachable!(),
                    }
                } else {
                    match key.direction {
                        LiveStreamDirection::Outbound => {
                            plain.project_outbound_requests(config, codecs, &key.group)
                        }
                        LiveStreamDirection::Inbound => {
                            plain.project_inbound_responses(config, codecs, &key.group)
                        }
                    }
                }
            }
            StreamBody::Http2(http2) => {
                http2.append_segment(segment);
                http2.project(config, codecs, &key.group, key.direction)
            }
        }
    }

    /// Convert a plain assembly into an HTTP/2 connection assembly once the
    /// buffered bytes are recognized as HTTP/2 frames.
    fn activate_http2(&mut self) {
        let StreamBody::Plain(plain) = &mut self.body else {
            return;
        };
        let mut http2 = Http2ConnectionAssembly::default();
        http2.frame_buffer = std::mem::take(&mut plain.buffer);
        http2.frame_base_offset = plain.base_offset;
        http2.frame_segments = std::mem::take(&mut plain.segments);
        // Re-route the already-buffered bytes through the frame parser. The
        // segment evidence moved into frame_segments is used per frame.
        http2.parse_frames();
        self.body = StreamBody::Http2(http2);
    }

    fn pending_request_marker(
        &self,
        key: &PayloadStreamGroupKey,
    ) -> Option<call::PendingLlmRequestMarker> {
        match &self.body {
            StreamBody::Plain(plain) => plain.pending_request_marker(key),
            StreamBody::Http2(http2) => http2.pending_request_marker(key),
        }
    }

    /// Take the plain assembly's in-flight response so trace-close
    /// finalization can materialize it. HTTP/2 streams finalize per stream and
    /// do not use the single in-flight slot.
    fn take_in_flight_response(&mut self) -> Option<InFlightResponse> {
        match &mut self.body {
            StreamBody::Plain(plain) => plain.in_flight_response.take(),
            StreamBody::Http2(_) => None,
        }
    }

    fn materialize_in_flight(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        message_start: usize,
    ) -> Option<(Vec<SemanticAction>, Vec<PayloadSegment>)> {
        match &mut self.body {
            StreamBody::Plain(plain) => {
                plain.materialize_in_flight(config, codecs, key, message_start)
            }
            StreamBody::Http2(_) => None,
        }
    }
}

fn looks_like_http2(bytes: &[u8]) -> bool {
    bytes.starts_with(HTTP2_CONNECTION_PREFACE) || decode_http2_frame(bytes).is_some()
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
}

fn response_completion_marker_seen(bytes: &[u8]) -> bool {
    contains_subslice(bytes, b"[DONE]")
        || contains_subslice(bytes, b"message_stop")
        || non_null_finish_reason_seen(bytes)
        || contains_subslice(bytes, b"event: done")
        || contains_subslice(bytes, b"event:done")
}

fn response_completion_tail(bytes: &[u8]) -> Vec<u8> {
    let marker_window = b"message_stop"
        .len()
        .max(b"[DONE]".len())
        .max(b"\"finish_reason\":null".len())
        .max(b"event: done".len())
        .max(b"event:done".len());
    let tail_len = marker_window.saturating_sub(1).min(bytes.len());
    bytes[bytes.len() - tail_len..].to_vec()
}

fn contains_subslice(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn non_null_finish_reason_seen(bytes: &[u8]) -> bool {
    const FINISH_REASON_KEY: &[u8] = b"\"finish_reason\"";

    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(offset) = find_subslice(&bytes[cursor..], FINISH_REASON_KEY) else {
            return false;
        };
        let value_start = cursor + offset + FINISH_REASON_KEY.len();
        if finish_reason_value_is_non_null(&bytes[value_start..]) {
            return true;
        }
        cursor = value_start;
    }
    false
}

fn finish_reason_value_is_non_null(bytes: &[u8]) -> bool {
    let mut cursor = skip_ascii_whitespace(bytes, 0);
    if bytes.get(cursor) != Some(&b':') {
        return false;
    }
    cursor += 1;
    cursor = skip_ascii_whitespace(bytes, cursor);
    if cursor >= bytes.len() {
        return false;
    }
    !bytes[cursor..].starts_with(b"null")
}

fn skip_ascii_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}

fn find_subslice(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
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

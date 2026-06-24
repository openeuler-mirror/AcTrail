//! Live LLM projection from retained plaintext payload segments.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

use config_core::daemon::SemanticRetentionConfig;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadSegment, PayloadSourceBoundary,
};
use model_core::process::ProcessIdentity;
use semantic_action::{
    LlmRequestContentWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionStatus, attr_keys as attrs,
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

#[derive(Default)]
pub(super) struct LiveLlmOutput {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) llm_request_contents: Vec<LlmRequestContentWrite>,
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
        let key = LiveStreamKey::from_segment(segment);
        let output = self
            .streams
            .entry(key.clone())
            .or_default()
            .observe_segment(&self.config, &key, segment);
        self.changed_actions(output)
    }

    pub(super) fn observe_http_message(&mut self, action: &SemanticAction) -> Vec<SemanticAction> {
        if !http::error_response(action) {
            return Vec::new();
        }
        let Some((request, call)) = self.take_open_request_for_http_response(action) else {
            return Vec::new();
        };
        let Some(failed_response) = http::failed_response_for_open_request(action, &request, &call)
        else {
            self.restore_open_request(request, call);
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
        let trace_close_completed_response_ids = self
            .open_action_versions
            .iter()
            .filter(|((candidate, _), action)| {
                *candidate == trace_id && response_completes_on_trace_close(action)
            })
            .map(|(_, action)| action.action_id.clone())
            .collect::<BTreeSet<_>>();
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
            let close_completed_successfully =
                trace_close_completes_action(action, &trace_close_completed_response_ids);
            if close_completed_successfully {
                action.status = SemanticActionStatus::Success;
                action.completeness = SemanticActionCompleteness::Complete;
                mark_trace_close_completion_attributes(action);
            } else {
                action.status = SemanticActionStatus::Error;
                action.completeness = SemanticActionCompleteness::Partial;
            }
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

    fn changed_actions(&mut self, output: LiveLlmOutput) -> LiveLlmOutput {
        let mut changed = LiveLlmOutput::default();
        let mut request_contents = output
            .llm_request_contents
            .into_iter()
            .map(|content| (content.manifest.action_id.clone(), content))
            .collect::<BTreeMap<_, _>>();
        for action in output.actions {
            let state_action = action_for_live_state(&action);
            if self.record_projected_action(&state_action) {
                if let Some(content) = request_contents.remove(&action.action_id) {
                    changed.llm_request_contents.push(content);
                }
                changed.actions.push(action.clone());
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
                            changed.actions.push(call);
                        }
                    } else {
                        let call = call::llm_call_from_request_response(&state_action, None);
                        self.open_calls_by_request.insert(
                            (state_action.trace_id, state_action.action_id.clone()),
                            call.clone(),
                        );
                        if self.record_projected_action(&call) {
                            changed.actions.push(call);
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
                            changed.actions.push(call);
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
    ) -> Option<(SemanticAction, SemanticAction)> {
        let response_order = LlmActionOrder {
            start_time: http_response.start_time,
            sequence_start: http_payload_sequence(http_response)?,
        };
        for stream_key in LlmStreamKey::from_http_response_candidates(http_response) {
            let Some(request) = self.take_open_request_before(&stream_key, response_order) else {
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

fn response_completes_on_trace_close(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::LlmResponse
        && action.status == SemanticActionStatus::InProgress
        && action
            .attributes
            .get(attrs::llm_response::PROVIDER_ID)
            .is_some_and(|value| value == "structured-json-sse")
        && action
            .attributes
            .get(attrs::llm_response::STREAM)
            .is_some_and(|value| value == "true")
        && llm_response_is_sse(action)
        && action
            .attributes
            .get(attrs::llm_response::DONE)
            .is_some_and(|value| value == "false")
        && successful_http_response(action)
        && llm_response_has_observed_output(action)
}

fn trace_close_completes_action(
    action: &SemanticAction,
    completed_response_ids: &BTreeSet<String>,
) -> bool {
    match action.kind {
        SemanticActionKind::LlmResponse => completed_response_ids.contains(&action.action_id),
        SemanticActionKind::SseStream => action
            .attributes
            .get(attrs::llm_response::ACTION_ID)
            .is_some_and(|response_id| completed_response_ids.contains(response_id)),
        SemanticActionKind::LlmCall => {
            action
                .attributes
                .get(attrs::llm_call::REQUEST_ACTION_ID)
                .is_some_and(|request_id| !request_id.is_empty())
                && action
                    .attributes
                    .get(attrs::llm_call::RESPONSE_ACTION_ID)
                    .is_some_and(|response_id| completed_response_ids.contains(response_id))
        }
        _ => false,
    }
}

fn mark_trace_close_completion_attributes(action: &mut SemanticAction) {
    match action.kind {
        SemanticActionKind::LlmResponse => {
            action
                .attributes
                .insert(attrs::llm_response::DONE.to_string(), "true".to_string());
        }
        SemanticActionKind::SseStream => {
            action
                .attributes
                .insert(attrs::sse::DONE.to_string(), "true".to_string());
        }
        _ => {}
    }
}

fn llm_response_is_sse(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(attrs::llm_response::BODY_FORMAT)
        .is_some_and(|value| value == "sse")
        || action
            .attributes
            .get(attrs::http_response::BODY_FORMAT)
            .is_some_and(|value| value == "sse")
}

fn successful_http_response(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(attrs::http_response::STATUS_CODE)
        .and_then(|value| value.parse::<u16>().ok())
        .is_some_and(|status| (200..300).contains(&status))
}

fn llm_response_has_observed_output(action: &SemanticAction) -> bool {
    positive_usize_attr(action, attrs::llm_response::CHUNK_COUNT)
        || attr_has_text(action, attrs::llm_response::CONTENT_TEXT)
        || attr_has_text(action, attrs::llm_response::REASONING_TEXT)
        || attr_has_text(action, attrs::llm_response::TOOL_CALLS_JSON)
}

fn attr_has_text(action: &SemanticAction, key: &str) -> bool {
    action
        .attributes
        .get(key)
        .is_some_and(|value| !value.is_empty())
}

fn positive_usize_attr(action: &SemanticAction, key: &str) -> bool {
    action
        .attributes
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|value| value > 0)
}

#[derive(Default)]
struct LiveStreamState {
    buffer: Vec<u8>,
    base_offset: usize,
    segments: VecDeque<LiveSegmentRange>,
    pending_raw_chunk_terminator: bool,
    completion_detector: ResponseCompletionDetector,
}

impl LiveStreamState {
    fn observe_segment(
        &mut self,
        config: &SemanticRetentionConfig,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
    ) -> LiveLlmOutput {
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
        key: &PayloadStreamGroupKey,
    ) -> LiveLlmOutput {
        self.discard_pending_raw_chunk_terminator();

        let mut output = LiveLlmOutput::default();
        while let Some(projection) = self.project_next_response(config, key) {
            let terminal = projection.terminal;
            let encoded_len = projection.encoded_len;
            output.actions.extend(projection.actions);
            if terminal {
                self.pending_raw_chunk_terminator = projection.raw_response;
                self.evict_encoded_len(encoded_len);
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
            start_time: first.segment.observed_at,
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

    #[cfg(test)]
    fn seen(&self) -> bool {
        self.seen
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

fn suppress_repeated_in_progress_response(
    existing: &SemanticAction,
    candidate: &SemanticAction,
) -> bool {
    if existing.kind != SemanticActionKind::LlmResponse
        || candidate.kind != SemanticActionKind::LlmResponse
        || existing.status != SemanticActionStatus::InProgress
        || candidate.status != SemanticActionStatus::InProgress
    {
        return false;
    }
    !llm_response_semantic_progress_changed(existing, candidate)
}

fn llm_response_semantic_progress_changed(
    existing: &SemanticAction,
    candidate: &SemanticAction,
) -> bool {
    const SEMANTIC_PROGRESS_ATTRS: &[&str] = &[
        attrs::llm_response::PROVIDER_ID,
        attrs::llm_response::MODEL,
        attrs::llm_response::CONTENT_TEXT,
        attrs::llm_response::REASONING_TEXT,
        attrs::llm_response::TOOL_CALLS_JSON,
        attrs::llm_response::CHUNK_COUNT,
        attrs::llm_response::DONE,
        attrs::llm_response::FINISH_REASON,
        attrs::llm_response::PROMPT_TOKENS,
        attrs::llm_response::COMPLETION_TOKENS,
        attrs::llm_response::TOTAL_TOKENS,
        attrs::llm_response::CACHED_PROMPT_TOKENS,
        attrs::llm_response::REASONING_TOKENS,
        attrs::llm_response::PROMPT_CACHE_HIT_TOKENS,
        attrs::llm_response::PROMPT_CACHE_MISS_TOKENS,
    ];

    SEMANTIC_PROGRESS_ATTRS
        .iter()
        .any(|key| existing.attributes.get(*key) != candidate.attributes.get(*key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_reason_null_sse_chunk_does_not_complete_response() {
        let mut detector = ResponseCompletionDetector::default();

        detector.observe(
            br#"data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}

"#,
        );

        assert!(!detector.seen());
    }

    #[test]
    fn explicit_response_completion_markers_complete_response() {
        for marker in [
            b"data: [DONE]\n\n".as_slice(),
            b"event: message_stop\n\n".as_slice(),
            b"event: done\n\n".as_slice(),
        ] {
            let mut detector = ResponseCompletionDetector::default();

            detector.observe(marker);

            assert!(
                detector.seen(),
                "marker {marker:?} should complete response"
            );
        }
    }

    #[test]
    fn split_done_event_marker_completes_response() {
        let mut detector = ResponseCompletionDetector::default();

        detector.observe(b"event: do");
        assert!(!detector.seen());

        detector.observe(b"ne\n\n");
        assert!(detector.seen());
    }
}

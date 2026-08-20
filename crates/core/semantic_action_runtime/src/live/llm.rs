//! Live LLM projection from retained plaintext payload segments.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

use config_core::daemon::{LlmAssemblyConfig, LlmRequestContentRetention, SemanticRetentionConfig};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSegment,
    PayloadSourceBoundary, PayloadStreamIdentity, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use semantic_action::{
    LlmRequestContentWrite, LlmRequestLineageWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionStatus, SemanticEvidenceKind, attr_keys as attrs,
};

use crate::payload_projection::http::{
    HTTP2_CONNECTION_PREFACE, HTTP2_CONTINUATION_FRAME_TYPE, HTTP2_DATA_FRAME_TYPE,
    HTTP2_FLAG_END_STREAM, HTTP2_HEADERS_FRAME_TYPE, decode_http2_frame, http1_request_starts_at,
    http1_response_starts_at, http2_data_payload,
};
use crate::payload_projection::llm::{
    InFlightResponse, IncrementalSseCache, LiveLlmProjection, LiveLlmResponseMessage,
    LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRegistry, PayloadStreamGroupKey,
    ProjectedLlmRequestHistory, ProjectedLlmToolResult, ProjectedProviderResponseId,
    live_llm_request_message_len, live_llm_request_prefix_skip_len, project_http2_stream_request,
    project_http2_stream_response, project_live_llm_request_message,
    project_live_llm_response_message, semantic_payload_draft,
};

use super::actions::action_for_live_state;
use super::http_exchange::{HttpResponseMatch, MatchedHttpRequest};

mod call;
mod http;
mod trajectory;
mod websocket;

use trajectory::{TrajectoryAssignment, TrajectoryClassification, TrajectoryClassifier};

pub(super) struct LiveLlmProjector {
    config: SemanticRetentionConfig,
    codecs: LlmCodecRegistry,
    streams: BTreeMap<LiveStreamKey, LiveStreamState>,
    open_requests: BTreeMap<LlmStreamKey, VecDeque<OpenLlmRequest>>,
    pending_responses: BTreeMap<LlmStreamKey, VecDeque<PendingLlmResponse>>,
    confirmed_http_exchanges: BTreeMap<LlmStreamKey, VecDeque<HttpResponseMatch>>,
    active_response_requests: BTreeMap<(TraceId, String), ActiveLlmResponseBinding>,
    max_confirmed_http_exchanges_per_stream: usize,
    assembly_limits: AssemblyLimits,
    pending_trajectory_actions: BTreeMap<(TraceId, String), PendingTrajectoryAction>,
    open_action_versions: BTreeMap<(TraceId, String), SemanticAction>,
    trajectory: Option<TrajectoryClassifier>,
    websocket: websocket::WebSocketLlmAdapter,
}

#[derive(Default)]
pub(super) struct LiveLlmOutput {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) llm_request_contents: Vec<LlmRequestContentWrite>,
    pub(super) llm_request_lineages: Vec<LlmRequestLineageWrite>,
    llm_request_histories: Vec<ProjectedLlmRequestHistory>,
    pub(super) llm_tool_results: Vec<ProjectedLlmToolResult>,
    provider_response_ids: Vec<ProjectedProviderResponseId>,
    non_reusable_response_ids: BTreeSet<String>,
    pub(super) payload_segments: Vec<PayloadSegment>,
    pub(super) http_request_links: Vec<LlmHttpRequestLink>,
}

pub(super) struct LlmHttpRequestLink {
    pub(super) llm_request: SemanticAction,
    pub(super) http_request: MatchedHttpRequest,
}

impl LiveLlmOutput {
    pub(super) fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.llm_request_lineages.extend(other.llm_request_lineages);
        self.llm_request_histories
            .extend(other.llm_request_histories);
        self.llm_tool_results.extend(other.llm_tool_results);
        self.provider_response_ids
            .extend(other.provider_response_ids);
        self.non_reusable_response_ids
            .extend(other.non_reusable_response_ids);
        self.payload_segments.extend(other.payload_segments);
        self.http_request_links.extend(other.http_request_links);
    }
}

impl LiveLlmProjector {
    pub(super) fn new(config: SemanticRetentionConfig) -> Self {
        if config.l0_llm_call.enabled
            && config.l0_llm_call.trajectory.enabled
            && !matches!(
                config.l0_llm_call.request_content,
                LlmRequestContentRetention::CanonicalBlocks
            )
        {
            tracing::warn!(
                request_content = ?config.l0_llm_call.request_content,
                "LLM trajectory identification is disabled because request content retention is not canonical_blocks"
            );
        }
        let trajectory = config
            .llm_trajectory_enabled()
            .then(|| TrajectoryClassifier::new(config.l0_llm_call.trajectory.into()));
        let websocket = websocket::WebSocketLlmAdapter::new(
            config.l0_llm_call.websocket_max_connections_per_process,
        );
        let assembly_limits = AssemblyLimits::from(config.l0_llm_call.assembly);
        let max_confirmed_http_exchanges_per_stream =
            usize::try_from(config.l2_http.exchange.max_pending_responses_per_stream)
                .expect("validated HTTP exchange response limit must fit usize");
        Self {
            config,
            codecs: LlmCodecRegistry::default(),
            streams: BTreeMap::new(),
            open_requests: BTreeMap::new(),
            pending_responses: BTreeMap::new(),
            confirmed_http_exchanges: BTreeMap::new(),
            active_response_requests: BTreeMap::new(),
            max_confirmed_http_exchanges_per_stream,
            assembly_limits,
            pending_trajectory_actions: BTreeMap::new(),
            open_action_versions: BTreeMap::new(),
            trajectory,
            websocket,
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
    sequence_start: u64,
    sequence_end: u64,
}

struct PendingLlmResponse {
    action: SemanticAction,
    provider_response_id: Option<String>,
}

struct ActiveLlmResponseBinding {
    request: SemanticAction,
    http_request_action_id: String,
    http_response_action_id: String,
}

struct PendingTrajectoryAction {
    action: SemanticAction,
    content: Option<LlmRequestContentWrite>,
    tool_results: Vec<ProjectedLlmToolResult>,
}

#[derive(Clone, Copy)]
struct AssemblyLimits {
    max_buffer_bytes: usize,
    max_segment_ranges: usize,
}

impl From<LlmAssemblyConfig> for AssemblyLimits {
    fn from(config: LlmAssemblyConfig) -> Self {
        Self {
            max_buffer_bytes: usize::try_from(config.max_buffer_bytes)
                .expect("validated LLM assembly byte limit must fit usize"),
            max_segment_ranges: usize::try_from(config.max_segment_ranges)
                .expect("validated LLM assembly segment limit must fit usize"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum AssemblyResetReason {
    BufferBytesExceeded,
    ConfirmedGap,
    OperationIncomplete,
    SegmentRangesExceeded,
}

impl AssemblyResetReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::BufferBytesExceeded => "buffer_bytes_exceeded",
            Self::ConfirmedGap => "confirmed_gap",
            Self::OperationIncomplete => "operation_incomplete",
            Self::SegmentRangesExceeded => "segment_ranges_exceeded",
        }
    }
}

impl OpenLlmRequest {
    fn matches_http_request(&self, request: &MatchedHttpRequest) -> bool {
        let method_matches = self
            .action
            .attributes
            .get(attrs::http_request::METHOD)
            .zip(request.method.as_ref())
            .is_some_and(|(left, right)| left == right);
        let target_matches = self
            .action
            .attributes
            .get(attrs::url::PATH)
            .zip(request.target.as_ref())
            .is_some_and(|(left, right)| left == right);
        if !method_matches || !target_matches {
            return false;
        }
        match request.stream_id.as_ref() {
            Some(stream_id) => {
                self.action.attributes.get(attrs::http_request::STREAM_ID) == Some(stream_id)
            }
            None => {
                self.sequence_start <= request.sequence && request.sequence <= self.sequence_end
            }
        }
    }
}

impl PendingLlmResponse {
    fn matches_http_response(&self, response: &SemanticAction) -> bool {
        if self.action.trace_id != response.trace_id || self.action.process != response.process {
            return false;
        }
        if self
            .action
            .attributes
            .get(attrs::http_response::STATUS_CODE)
            .zip(response.attributes.get("status_code"))
            .is_some_and(|(left, right)| left != right)
        {
            return false;
        }
        match (
            self.action.attributes.get(attrs::http_response::STREAM_ID),
            response.attributes.get("stream_id"),
        ) {
            (Some(left), Some(right)) => return left == right,
            (None, None) => {}
            _ => return false,
        }
        let Some(sequence) = response
            .attributes
            .get("payload_sequence")
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return false;
        };
        call::payload_sequence_start(&self.action)
            .zip(call::payload_sequence_end(&self.action))
            .is_some_and(|(start, end)| start <= sequence && sequence <= end)
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
}

impl LiveLlmProjector {
    pub(super) fn observe_payload_segment(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        if !self.config.llm_layer_enabled() {
            return LiveLlmOutput::default();
        }
        if !plaintext_http_candidate(segment) {
            return LiveLlmOutput::default();
        }
        let websocket = self.websocket.observe(segment);
        let mut changed = self.observe_http_payload(segment);
        for candidate in &websocket.projected {
            changed.extend(self.observe_http_payload(candidate));
        }
        self.forget_websocket_exchange_streams(
            segment.trace_id,
            &segment.process,
            &websocket.forgotten_exchange_streams,
        );
        changed
    }

    pub(super) fn observe_payload_gap(&mut self, segment: &PayloadSegment) -> LiveLlmOutput {
        if !self.config.llm_layer_enabled() || !plaintext_http_candidate(segment) {
            return LiveLlmOutput::default();
        }
        let key = LiveStreamKey::from_segment(segment);
        let output = self
            .streams
            .entry(key.clone())
            .or_default()
            .reset_for_discontinuity(
                &self.config,
                &self.codecs,
                &key,
                segment,
                AssemblyResetReason::ConfirmedGap,
                false,
            );
        let changed = self.changed_actions(output);
        self.forget_payload_associations(segment);
        changed
    }

    pub(super) fn forget_payload_associations(&mut self, segment: &PayloadSegment) {
        self.forget_payload_associations_by_identity(&PayloadStreamIdentity::from_segment(segment));
    }

    pub(super) fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        let stream_key = identity.stream_key.to_string();
        self.streams.retain(|key, _| {
            key.group.trace_id != identity.trace_id
                || key.group.process != identity.process
                || key.group.stream_key != stream_key
        });
        self.forget_payload_associations_by_identity(identity);
    }

    fn forget_payload_associations_by_identity(&mut self, identity: &PayloadStreamIdentity) {
        let stream_key = identity.stream_key.to_string();
        let matches_key = |key: &LlmStreamKey| {
            key.trace_id == identity.trace_id
                && key.process == identity.process
                && key.stream_key == stream_key
        };
        self.open_requests.retain(|key, _| !matches_key(key));
        self.pending_responses.retain(|key, _| !matches_key(key));
        self.confirmed_http_exchanges
            .retain(|key, _| !matches_key(key));
        self.active_response_requests.retain(|_, binding| {
            binding.request.trace_id != identity.trace_id
                || binding.request.process != identity.process
                || binding.request.attributes.get(attrs::payload::STREAM_KEY) != Some(&stream_key)
        });
    }

    fn forget_websocket_exchange_streams(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        prefixes: &[websocket::WebSocketExchangeStreamPrefix],
    ) {
        if prefixes.is_empty() {
            return;
        }
        let matches_stream =
            |candidate_trace: TraceId, candidate_process: &ProcessIdentity, stream_key: &str| {
                candidate_trace == trace_id
                    && candidate_process == process
                    && prefixes.iter().any(|prefix| prefix.matches(stream_key))
            };
        self.streams.retain(|key, _| {
            !matches_stream(
                key.group.trace_id,
                &key.group.process,
                &key.group.stream_key,
            )
        });
        self.open_requests
            .retain(|key, _| !matches_stream(key.trace_id, &key.process, &key.stream_key));
        self.pending_responses
            .retain(|key, _| !matches_stream(key.trace_id, &key.process, &key.stream_key));
        self.confirmed_http_exchanges
            .retain(|key, _| !matches_stream(key.trace_id, &key.process, &key.stream_key));
        self.active_response_requests.retain(|_, binding| {
            binding
                .request
                .attributes
                .get(attrs::payload::STREAM_KEY)
                .is_none_or(|stream_key| {
                    !matches_stream(
                        binding.request.trace_id,
                        &binding.request.process,
                        stream_key,
                    )
                })
        });
    }

    pub(super) fn observe_http_exchange(&mut self, matched: &HttpResponseMatch) -> LiveLlmOutput {
        if !http::terminal_failure_response(&matched.response) {
            let Some(stream_key) = self.remember_confirmed_http_exchange(matched.clone()) else {
                return LiveLlmOutput::default();
            };
            return self.reconcile_confirmed_http_exchanges(&stream_key);
        }
        let action = &matched.response;
        let matched_request = &matched.request;
        if action
            .attributes
            .get(attrs::http_response::REQUEST_ACTION_ID)
            != Some(&matched_request.action_id)
        {
            return LiveLlmOutput::default();
        }
        let Some(request) = self.take_open_request_for_http_response(action, matched_request)
        else {
            return LiveLlmOutput::default();
        };
        let call = call::llm_call_from_request_response(&request, None);
        let Some(mut failed_response) =
            http::failed_response_for_open_request(action, &request, &call)
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        failed_response.attributes.insert(
            attrs::http_response::REQUEST_ACTION_ID.to_string(),
            matched_request.action_id.clone(),
        );
        let mut failed_call =
            call::llm_call_from_request_response(&request, Some(&failed_response));
        failed_call.attributes.insert(
            attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
            action.action_id.clone(),
        );
        let mut output = LiveLlmOutput::default();
        output.http_request_links.push(LlmHttpRequestLink {
            llm_request: request.clone(),
            http_request: matched_request.clone(),
        });
        if self.record_projected_action(&failed_response) {
            output.actions.push(failed_response);
        }
        if self.record_projected_action(&failed_call) {
            output.actions.push(failed_call);
        }
        output
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        if let Some(trajectory) = self.trajectory.as_mut() {
            trajectory.forget_trace(trace_id);
        }
        self.websocket.forget_trace(trace_id);
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.open_requests.retain(|key, _| key.trace_id != trace_id);
        self.pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.confirmed_http_exchanges
            .retain(|key, _| key.trace_id != trace_id);
        self.active_response_requests
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_trajectory_actions
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.open_action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        self.websocket.forget_trace(trace_id);
        let mut output = LiveLlmOutput::default();
        if let Some(classifier) = self.trajectory.as_mut() {
            let assignments = classifier.finalize_trace(trace_id);
            self.apply_resolved_trajectory_assignments(trace_id, assignments, &mut output);
        }
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
            output.payload_segments.extend(drafts);
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
                if let Some(stream_key) = LlmStreamKey::from_llm_response(action) {
                    self.remember_pending_response(action.clone(), None);
                    let reconciled = self.reconcile_confirmed_http_exchanges(&stream_key);
                    if reconciled
                        .actions
                        .iter()
                        .any(|candidate| candidate.kind == SemanticActionKind::LlmCall)
                    {
                        output.extend(reconciled);
                        continue;
                    }
                }
                output.actions.push(action.clone());
                if let Some(request) = self.request_for_in_flight_response(action) {
                    let mut call = call::llm_call_from_request_response(&request, Some(action));
                    call.status = SemanticActionStatus::Error;
                    call.completeness = SemanticActionCompleteness::Partial;
                    call.end_time = Some(finished_at);
                    call.attributes.insert(
                        attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                        "true".to_string(),
                    );
                    output.actions.push(call);
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
            output.actions.push(call);
        }
        self.streams.retain(|key, _| key.group.trace_id != trace_id);
        self.open_requests.retain(|key, _| key.trace_id != trace_id);
        self.pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.confirmed_http_exchanges
            .retain(|key, _| key.trace_id != trace_id);
        self.active_response_requests
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_trajectory_actions
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.open_action_versions
            .retain(|(candidate, _), _| *candidate != trace_id);
        output
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
            .observe_segment(
                &self.config,
                &self.codecs,
                self.assembly_limits,
                &key,
                segment,
            );
        self.changed_actions(output)
    }

    fn changed_actions(&mut self, output: LiveLlmOutput) -> LiveLlmOutput {
        let mut changed = LiveLlmOutput::default();
        let non_reusable_response_ids = output.non_reusable_response_ids;
        let mut request_contents = output
            .llm_request_contents
            .into_iter()
            .map(|content| (content.manifest.action_id.clone(), content))
            .collect::<BTreeMap<_, _>>();
        let mut request_histories = output
            .llm_request_histories
            .into_iter()
            .map(|history| (history.action_id.clone(), history))
            .collect::<BTreeMap<_, _>>();
        let mut tool_results = BTreeMap::<String, Vec<ProjectedLlmToolResult>>::new();
        for result in output.llm_tool_results {
            tool_results
                .entry(result.request_action_id.clone())
                .or_default()
                .push(result);
        }
        let mut provider_response_ids = output
            .provider_response_ids
            .into_iter()
            .map(|metadata| (metadata.action_id, metadata.provider_response_id))
            .collect::<BTreeMap<_, _>>();
        for mut action in output.actions {
            if !self.config.l4_payload.enabled {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            let mut deferred_trajectory = false;
            let mut resolved_trajectories = Vec::new();
            let lineage = if action.kind == SemanticActionKind::LlmRequest {
                self.trajectory.as_mut().and_then(|classifier| {
                    let classification = match request_histories.remove(&action.action_id) {
                        Some(history) => classifier.classify(
                            action.trace_id,
                            action.process,
                            history,
                            action.start_time,
                        ),
                        None => TrajectoryClassification::Assigned(
                            classifier.classify_failure(action.action_id.clone()),
                        ),
                    };
                    resolved_trajectories = classifier.take_resolved();
                    let TrajectoryClassification::Assigned(assignment) = classification else {
                        deferred_trajectory = true;
                        return None;
                    };
                    action.attributes.insert(
                        attrs::llm_request::TRAJECTORY_ID.to_string(),
                        assignment.trajectory_id.clone(),
                    );
                    action.attributes.insert(
                        attrs::llm_request::TRAJECTORY_INFERENCE_VERSION.to_string(),
                        assignment.inference_version.to_string(),
                    );
                    Some(assignment.lineage(action.trace_id))
                })
            } else {
                None
            };
            let mut state_action = action_for_live_state(&action);
            if state_action.kind == SemanticActionKind::LlmResponse {
                if let Some(binding) = self
                    .active_response_requests
                    .get(&(state_action.trace_id, state_action.action_id.clone()))
                {
                    state_action.attributes.insert(
                        attrs::http_response::REQUEST_ACTION_ID.to_string(),
                        binding.http_request_action_id.clone(),
                    );
                    action.attributes.insert(
                        attrs::http_response::REQUEST_ACTION_ID.to_string(),
                        binding.http_request_action_id.clone(),
                    );
                }
            }
            self.apply_resolved_trajectory_assignments(
                state_action.trace_id,
                resolved_trajectories,
                &mut changed,
            );
            if deferred_trajectory {
                self.pending_trajectory_actions.insert(
                    (state_action.trace_id, state_action.action_id.clone()),
                    PendingTrajectoryAction {
                        action: state_action.clone(),
                        content: request_contents.remove(&state_action.action_id),
                        tool_results: tool_results
                            .remove(&state_action.action_id)
                            .unwrap_or_default(),
                    },
                );
            }
            let action_changed = self.record_projected_action(&state_action);
            if action_changed && !deferred_trajectory {
                if let Some(content) = request_contents.remove(&action.action_id) {
                    changed.llm_request_contents.push(content);
                }
                if let Some(results) = tool_results.remove(&action.action_id) {
                    changed.llm_tool_results.extend(results);
                }
                if let Some(lineage) = lineage {
                    changed.llm_request_lineages.push(lineage);
                }
                changed.actions.push(action);
            }
            match state_action.kind {
                SemanticActionKind::LlmRequest => {
                    self.remember_open_request(state_action.clone());
                    if let Some(stream_key) = LlmStreamKey::from_llm_request(&state_action) {
                        changed.extend(self.reconcile_exact_websocket_exchange(&stream_key));
                        changed.extend(self.reconcile_confirmed_http_exchanges(&stream_key));
                    }
                }
                SemanticActionKind::LlmResponse => {
                    let provider_response_id =
                        provider_response_ids.remove(&state_action.action_id);
                    if let Some(binding) = self.request_for_response_update(&state_action) {
                        let assignments = self.register_provider_response(
                            &binding.request,
                            provider_response_id.as_deref(),
                            state_action.end_time.unwrap_or(state_action.start_time),
                        );
                        self.apply_resolved_trajectory_assignments(
                            binding.request.trace_id,
                            assignments,
                            &mut changed,
                        );
                        let mut call = call::llm_call_from_request_response(
                            &binding.request,
                            Some(&state_action),
                        );
                        call.attributes.insert(
                            attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
                            binding.http_response_action_id.clone(),
                        );
                        self.update_active_response_request(&state_action, binding);
                        self.record_derived_call(call, &mut changed.actions);
                    } else if !non_reusable_response_ids.contains(&state_action.action_id) {
                        self.remember_pending_response(state_action.clone(), provider_response_id);
                        if let Some(stream_key) = LlmStreamKey::from_llm_response(&state_action) {
                            changed.extend(self.reconcile_exact_websocket_exchange(&stream_key));
                            changed.extend(self.reconcile_confirmed_http_exchanges(&stream_key));
                        }
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

    fn register_provider_response(
        &mut self,
        request: &SemanticAction,
        provider_response_id: Option<&str>,
        observed_at: SystemTime,
    ) -> Vec<TrajectoryAssignment> {
        let Some(provider_response_id) = provider_response_id else {
            return Vec::new();
        };
        let Some(classifier_id) = request.attributes.get(attrs::llm_request::CLASSIFIER_ID) else {
            return Vec::new();
        };
        if let Some(classifier) = self.trajectory.as_mut() {
            return classifier.register_provider_response(
                request.trace_id,
                request.process.clone(),
                classifier_id.clone(),
                &request.action_id,
                provider_response_id,
                observed_at,
            );
        }
        Vec::new()
    }

    fn apply_resolved_trajectory_assignments(
        &mut self,
        trace_id: TraceId,
        assignments: Vec<TrajectoryAssignment>,
        changed: &mut LiveLlmOutput,
    ) {
        for assignment in assignments {
            let Some(pending) = self
                .pending_trajectory_actions
                .remove(&(trace_id, assignment.action_id.clone()))
            else {
                continue;
            };
            let mut action = pending.action;
            action.attributes.insert(
                attrs::llm_request::TRAJECTORY_ID.to_string(),
                assignment.trajectory_id.clone(),
            );
            action.attributes.insert(
                attrs::llm_request::TRAJECTORY_INFERENCE_VERSION.to_string(),
                assignment.inference_version.to_string(),
            );
            changed
                .llm_request_lineages
                .push(assignment.lineage(action.trace_id));
            if let Some(content) = pending.content {
                changed.llm_request_contents.push(content);
            }
            changed.llm_tool_results.extend(pending.tool_results);
            self.update_open_request(&action);
            if self.record_projected_action(&action) {
                changed.actions.push(action);
            }
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
        let Some(sequence_end) = call::payload_sequence_end(&request) else {
            return;
        };
        let requests = self.open_requests.entry(stream_key).or_default();
        if let Some(existing) = requests
            .iter_mut()
            .find(|candidate| candidate.action.action_id == request.action_id)
        {
            existing.action = request;
            existing.sequence_start = sequence_start;
            existing.sequence_end = sequence_end;
        } else {
            requests.push_back(OpenLlmRequest {
                action: request,
                sequence_start,
                sequence_end,
            });
        }
    }

    fn update_open_request(&mut self, request: &SemanticAction) {
        let Some(stream_key) = LlmStreamKey::from_llm_request(request) else {
            return;
        };
        let Some(requests) = self.open_requests.get_mut(&stream_key) else {
            return;
        };
        if let Some(existing) = requests
            .iter_mut()
            .find(|candidate| candidate.action.action_id == request.action_id)
        {
            existing.action = request.clone();
        }
    }

    fn restore_open_request(&mut self, request: SemanticAction) {
        self.remember_open_request(request);
    }

    fn remember_pending_response(
        &mut self,
        response: SemanticAction,
        provider_response_id: Option<String>,
    ) {
        let Some(stream_key) = LlmStreamKey::from_llm_response(&response) else {
            return;
        };
        let responses = self.pending_responses.entry(stream_key).or_default();
        if let Some(existing) = responses
            .iter()
            .position(|candidate| candidate.action.action_id == response.action_id)
        {
            let previous_provider_response_id = responses[existing].provider_response_id.take();
            responses[existing] = PendingLlmResponse {
                action: response,
                provider_response_id: provider_response_id.or(previous_provider_response_id),
            };
            return;
        }
        responses.push_back(PendingLlmResponse {
            action: response,
            provider_response_id,
        });
    }

    fn remember_confirmed_http_exchange(
        &mut self,
        matched: HttpResponseMatch,
    ) -> Option<LlmStreamKey> {
        let candidates = LlmStreamKey::from_http_response_candidates(&matched.response);
        let stream_key = candidates
            .iter()
            .find(|candidate| {
                self.open_requests.contains_key(*candidate)
                    && self.pending_responses.contains_key(*candidate)
            })
            .or_else(|| {
                candidates.iter().find(|candidate| {
                    self.open_requests.contains_key(*candidate)
                        || self.pending_responses.contains_key(*candidate)
                })
            })
            .or_else(|| candidates.first())?
            .clone();
        let exchanges = self
            .confirmed_http_exchanges
            .entry(stream_key.clone())
            .or_default();
        if let Some(existing) = exchanges
            .iter_mut()
            .find(|exchange| exchange.response.action_id == matched.response.action_id)
        {
            *existing = matched;
            return Some(stream_key);
        }
        if exchanges.len() >= self.max_confirmed_http_exchanges_per_stream {
            exchanges.pop_front();
            tracing::warn!(
                trace_id = stream_key.trace_id.get(),
                process_id = stream_key.process.get(),
                stream_key = %stream_key.stream_key,
                stream_id = ?stream_key.http_stream_id,
                "dropped oldest unconsumed confirmed HTTP exchange at configured capacity"
            );
        }
        exchanges.push_back(matched);
        Some(stream_key)
    }

    fn request_for_response_update(
        &mut self,
        response: &SemanticAction,
    ) -> Option<ActiveLlmResponseBinding> {
        let key = (response.trace_id, response.action_id.clone());
        self.active_response_requests.remove(&key)
    }

    fn update_active_response_request(
        &mut self,
        response: &SemanticAction,
        binding: ActiveLlmResponseBinding,
    ) {
        let key = (response.trace_id, response.action_id.clone());
        if response.status == SemanticActionStatus::InProgress {
            self.active_response_requests.insert(key, binding);
        } else {
            self.active_response_requests.remove(&key);
        }
    }

    fn request_for_in_flight_response(&self, response: &SemanticAction) -> Option<SemanticAction> {
        self.active_response_requests
            .get(&(response.trace_id, response.action_id.clone()))
            .map(|binding| binding.request.clone())
    }

    fn reconcile_confirmed_http_exchanges(&mut self, stream_key: &LlmStreamKey) -> LiveLlmOutput {
        let selection = self
            .confirmed_http_exchanges
            .get(stream_key)
            .and_then(|exchanges| {
                exchanges
                    .iter()
                    .enumerate()
                    .find_map(|(exchange_index, exchange)| {
                        let request_index = self
                            .open_requests
                            .get(stream_key)?
                            .iter()
                            .position(|request| request.matches_http_request(&exchange.request))?;
                        let response_index =
                            self.pending_responses.get(stream_key)?.iter().position(
                                |response| response.matches_http_response(&exchange.response),
                            )?;
                        Some((exchange_index, request_index, response_index))
                    })
            });
        let Some((exchange_index, request_index, response_index)) = selection else {
            return LiveLlmOutput::default();
        };
        let Some(exchange) = self
            .confirmed_http_exchanges
            .get_mut(stream_key)
            .and_then(|exchanges| exchanges.remove(exchange_index))
        else {
            return LiveLlmOutput::default();
        };
        let Some(request) = self
            .open_requests
            .get_mut(stream_key)
            .and_then(|requests| requests.remove(request_index))
            .map(|request| request.action)
        else {
            return LiveLlmOutput::default();
        };
        let Some(mut response) = self
            .pending_responses
            .get_mut(stream_key)
            .and_then(|responses| responses.remove(response_index))
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        if self
            .open_requests
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.open_requests.remove(stream_key);
        }
        if self
            .pending_responses
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.pending_responses.remove(stream_key);
        }
        if self
            .confirmed_http_exchanges
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.confirmed_http_exchanges.remove(stream_key);
        }
        let http_request_action_id = exchange.request.action_id;
        let http_request_evidence = exchange.request.evidence;
        let http_response_action_id = exchange.response.action_id;
        response.action.attributes.insert(
            attrs::http_response::REQUEST_ACTION_ID.to_string(),
            http_request_action_id.clone(),
        );
        let mut output = LiveLlmOutput::default();
        output.http_request_links.push(LlmHttpRequestLink {
            llm_request: request.clone(),
            http_request: MatchedHttpRequest {
                action_id: http_request_action_id.clone(),
                evidence: http_request_evidence,
                sequence: exchange.request.sequence,
                method: exchange.request.method,
                target: exchange.request.target,
                stream_id: exchange.request.stream_id,
            },
        });
        let assignments = self.register_provider_response(
            &request,
            response.provider_response_id.as_deref(),
            response
                .action
                .end_time
                .unwrap_or(response.action.start_time),
        );
        self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
        if self.record_projected_action(&response.action) {
            output.actions.push(response.action.clone());
        }
        let mut call = call::llm_call_from_request_response(&request, Some(&response.action));
        if let Some(finalized) = response
            .action
            .attributes
            .get(attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE)
        {
            call.attributes.insert(
                attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                finalized.clone(),
            );
        }
        call.attributes.insert(
            attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
            http_response_action_id.clone(),
        );
        self.update_active_response_request(
            &response.action,
            ActiveLlmResponseBinding {
                request,
                http_request_action_id,
                http_response_action_id,
            },
        );
        self.record_derived_call(call, &mut output.actions);
        output
    }

    fn reconcile_exact_websocket_exchange(&mut self, stream_key: &LlmStreamKey) -> LiveLlmOutput {
        if !websocket::WebSocketLlmAdapter::is_exchange_stream_key(&stream_key.stream_key) {
            return LiveLlmOutput::default();
        }
        let Some(request) = self
            .open_requests
            .get_mut(stream_key)
            .and_then(VecDeque::pop_front)
            .map(|request| request.action)
        else {
            return LiveLlmOutput::default();
        };
        let Some(response) = self
            .pending_responses
            .get_mut(stream_key)
            .and_then(VecDeque::pop_front)
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        if self
            .open_requests
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.open_requests.remove(stream_key);
        }
        if self
            .pending_responses
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.pending_responses.remove(stream_key);
        }

        let mut output = LiveLlmOutput::default();
        let assignments = self.register_provider_response(
            &request,
            response.provider_response_id.as_deref(),
            response
                .action
                .end_time
                .unwrap_or(response.action.start_time),
        );
        self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
        if self.record_projected_action(&response.action) {
            output.actions.push(response.action.clone());
        }
        let call = call::llm_call_from_request_response(&request, Some(&response.action));
        self.record_derived_call(call, &mut output.actions);
        output
    }

    fn take_open_request_for_http_response(
        &mut self,
        http_response: &SemanticAction,
        matched_request: &MatchedHttpRequest,
    ) -> Option<SemanticAction> {
        for stream_key in LlmStreamKey::from_http_response_candidates(http_response) {
            let Some(requests) = self.open_requests.get_mut(&stream_key) else {
                continue;
            };
            if !requests
                .front()
                .is_some_and(|request| request.matches_http_request(matched_request))
            {
                continue;
            }
            let request = requests.pop_front()?.action;
            if requests.is_empty() {
                self.open_requests.remove(&stream_key);
            }
            return Some(request);
        }
        None
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
    fn admission_failure(
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
            output
                .llm_request_histories
                .extend(projection.llm_request_histories);
            output.llm_tool_results.extend(projection.llm_tool_results);
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
            output
                .provider_response_ids
                .extend(projection.provider_response_ids);
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
        if self.buffer.is_empty() {
            self.buffer = Vec::new();
            self.segments = VecDeque::new();
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
        output
            .llm_request_histories
            .extend(projection.llm_request_histories);
        output.llm_tool_results.extend(projection.llm_tool_results);
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
        output
            .provider_response_ids
            .extend(projection.provider_response_ids);
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
    discarded_streams: BTreeSet<u32>,
}

impl Default for Http2ConnectionAssembly {
    fn default() -> Self {
        Self {
            frame_buffer: Vec::new(),
            frame_base_offset: 0,
            frame_segments: VecDeque::new(),
            streams: BTreeMap::new(),
            discarded_streams: BTreeSet::new(),
        }
    }
}

impl Http2ConnectionAssembly {
    fn admission_failure(
        &self,
        segment: &PayloadSegment,
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        let buffered_stream_bytes = self
            .streams
            .values()
            .map(|stream| stream.plain.buffer.len())
            .try_fold(0_usize, usize::checked_add);
        if buffered_stream_bytes
            .and_then(|bytes| bytes.checked_add(self.frame_buffer.len()))
            .and_then(|bytes| bytes.checked_add(segment.bytes.len()))
            .is_none_or(|bytes| bytes > limits.max_buffer_bytes)
        {
            return Some(AssemblyResetReason::BufferBytesExceeded);
        }
        let stream_ranges = self
            .streams
            .values()
            .map(|stream| stream.plain.segments.len())
            .try_fold(0_usize, usize::checked_add);
        stream_ranges
            .and_then(|ranges| ranges.checked_add(self.frame_segments.len()))
            .and_then(|ranges| ranges.checked_add(self.discarded_streams.len()))
            .and_then(|ranges| ranges.checked_add(1))
            .is_none_or(|ranges| ranges > limits.max_segment_ranges)
            .then_some(AssemblyResetReason::SegmentRangesExceeded)
    }

    fn append_segment(
        &mut self,
        segment: &PayloadSegment,
        limits: AssemblyLimits,
    ) -> Vec<AssemblyResetReason> {
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
        self.parse_frames(limits)
    }

    fn parse_frames(&mut self, limits: AssemblyLimits) -> Vec<AssemblyResetReason> {
        let mut resets = Vec::new();
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
                        if let Some(reason) =
                            self.route_stream_data(stream_id, frame_start, data, limits)
                        {
                            resets.push(reason);
                        }
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
        resets
    }

    fn route_stream_data(
        &mut self,
        stream_id: u32,
        frame_start: usize,
        data: &[u8],
        limits: AssemblyLimits,
    ) -> Option<AssemblyResetReason> {
        if self.discarded_streams.contains(&stream_id) {
            return None;
        }
        let Some(segment) = self.segment_metadata_at(frame_start).cloned() else {
            return None;
        };
        let stream = self.streams.entry(stream_id).or_default();
        if let Some(reason) = stream.plain.admission_failure(data.len(), 1, limits) {
            self.streams.remove(&stream_id);
            self.discarded_streams.insert(stream_id);
            return Some(reason);
        }
        stream.plain.append_plaintext(data, segment);
        None
    }

    fn segment_metadata_at(&self, global_offset: usize) -> Option<&PayloadSegment> {
        self.frame_segments
            .iter()
            .find(|range| range.start <= global_offset && global_offset < range.end)
            .map(|range| &range.segment)
    }

    fn mark_end_stream(&mut self, stream_id: u32) {
        if self.discarded_streams.remove(&stream_id) {
            return;
        }
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
        if self.frame_buffer.is_empty() {
            self.frame_buffer = Vec::new();
            self.frame_segments = VecDeque::new();
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
}

/// The byte-stream assembly for one (stream_key, direction): either a plain
/// sequential stream (HTTP/1, raw) or a de-multiplexed HTTP/2 connection.
enum StreamBody {
    Plain(PlainStreamAssembly),
    Http2(Http2ConnectionAssembly),
}

struct LiveStreamState {
    body: StreamBody,
    desynchronized: bool,
}

impl Default for LiveStreamState {
    fn default() -> Self {
        Self {
            body: StreamBody::Plain(PlainStreamAssembly::default()),
            desynchronized: false,
        }
    }
}

impl LiveStreamState {
    fn observe_segment(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        limits: AssemblyLimits,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
    ) -> LiveLlmOutput {
        if let Some(reason) = incomplete_segment_reason(segment) {
            return self.reset_for_discontinuity(config, codecs, key, segment, reason, true);
        }
        if self.desynchronized {
            if !trusted_resynchronization_boundary(key.direction, &segment.bytes) {
                self.discard_segment(segment);
                return LiveLlmOutput::default();
            }
            self.desynchronized = false;
            tracing::warn!(
                trace_id = key.group.trace_id.get(),
                process_id = key.group.process.get(),
                stream_key = %key.group.stream_key,
                direction = ?key.direction,
                "LLM plaintext assembly resynchronized at a trusted HTTP boundary"
            );
        }
        match &mut self.body {
            StreamBody::Plain(plain) => {
                if let Some(reason) = plain.admission_failure(segment.bytes.len(), 1, limits) {
                    return self
                        .reset_for_discontinuity(config, codecs, key, segment, reason, true);
                }
                plain.append_segment(segment);
                if looks_like_http2(&plain.buffer) {
                    let resets = self.activate_http2(limits);
                    log_http2_stream_resets(key, &resets);
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
                if let Some(reason) = http2.admission_failure(segment, limits) {
                    return self
                        .reset_for_discontinuity(config, codecs, key, segment, reason, true);
                }
                let resets = http2.append_segment(segment, limits);
                log_http2_stream_resets(key, &resets);
                http2.project(config, codecs, &key.group, key.direction)
            }
        }
    }

    /// Convert a plain assembly into an HTTP/2 connection assembly once the
    /// buffered bytes are recognized as HTTP/2 frames.
    fn activate_http2(&mut self, limits: AssemblyLimits) -> Vec<AssemblyResetReason> {
        let StreamBody::Plain(plain) = &mut self.body else {
            return Vec::new();
        };
        let mut http2 = Http2ConnectionAssembly::default();
        http2.frame_buffer = std::mem::take(&mut plain.buffer);
        http2.frame_base_offset = plain.base_offset;
        http2.frame_segments = std::mem::take(&mut plain.segments);
        // Re-route the already-buffered bytes through the frame parser. The
        // segment evidence moved into frame_segments is used per frame.
        let resets = http2.parse_frames(limits);
        self.body = StreamBody::Http2(http2);
        resets
    }

    fn reset_for_discontinuity(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &LiveStreamKey,
        segment: &PayloadSegment,
        reason: AssemblyResetReason,
        discard_segment: bool,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        if let Some(in_flight) = self.take_in_flight_response()
            && let Some((mut actions, drafts)) =
                self.materialize_in_flight(config, codecs, &key.group, in_flight.message_start)
        {
            output.payload_segments.extend(drafts);
            for action in &mut actions {
                if action.kind != SemanticActionKind::LlmResponse {
                    continue;
                }
                action.status = SemanticActionStatus::Error;
                action.completeness = SemanticActionCompleteness::Partial;
                action.end_time = Some(segment.observed_at);
                output
                    .non_reusable_response_ids
                    .insert(action.action_id.clone());
            }
            output.actions.extend(actions);
        }
        let buffered_bytes = self.buffered_bytes();
        if discard_segment {
            self.discard_segment(segment);
        } else {
            let base_offset = self.stream_end_offset();
            self.body = StreamBody::Plain(PlainStreamAssembly {
                base_offset,
                ..PlainStreamAssembly::default()
            });
        }
        self.desynchronized = true;
        tracing::warn!(
            trace_id = key.group.trace_id.get(),
            process_id = key.group.process.get(),
            stream_key = %key.group.stream_key,
            direction = ?key.direction,
            reason = reason.as_str(),
            buffered_bytes,
            "discarded unsafe LLM plaintext assembly state"
        );
        output
    }

    fn discard_segment(&mut self, segment: &PayloadSegment) {
        let next_offset = self
            .stream_end_offset()
            .saturating_add(usize::try_from(segment.original_size).unwrap_or(usize::MAX));
        self.body = StreamBody::Plain(PlainStreamAssembly {
            base_offset: next_offset,
            ..PlainStreamAssembly::default()
        });
    }

    fn stream_end_offset(&self) -> usize {
        match &self.body {
            StreamBody::Plain(plain) => plain.base_offset.saturating_add(plain.buffer.len()),
            StreamBody::Http2(http2) => http2
                .frame_base_offset
                .saturating_add(http2.frame_buffer.len()),
        }
    }

    fn buffered_bytes(&self) -> usize {
        match &self.body {
            StreamBody::Plain(plain) => plain.buffer.len(),
            StreamBody::Http2(http2) => {
                http2.frame_buffer.len()
                    + http2
                        .streams
                        .values()
                        .map(|stream| stream.plain.buffer.len())
                        .sum::<usize>()
            }
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

fn incomplete_segment_reason(segment: &PayloadSegment) -> Option<AssemblyResetReason> {
    (segment.truncation == PayloadTruncationState::Truncated
        || matches!(
            segment.operation_completion_state,
            PayloadOperationCompletionState::Partial | PayloadOperationCompletionState::Failed
        )
        || segment.operation_original_size != segment.operation_captured_size)
        .then_some(AssemblyResetReason::OperationIncomplete)
}

fn trusted_resynchronization_boundary(direction: LiveStreamDirection, bytes: &[u8]) -> bool {
    bytes.starts_with(HTTP2_CONNECTION_PREFACE)
        || match direction {
            LiveStreamDirection::Outbound => http1_request_starts_at(bytes),
            LiveStreamDirection::Inbound => http1_response_starts_at(bytes),
        }
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

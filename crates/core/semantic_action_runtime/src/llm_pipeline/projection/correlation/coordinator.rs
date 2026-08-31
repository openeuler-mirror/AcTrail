//! Owned correlation state for LLM requests, responses, and HTTP exchanges.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::SystemTime;

use config_core::daemon::LlmProjectionStateConfig;
use model_core::ids::TraceId;
use model_core::payload::PayloadStreamIdentity;
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, attr_keys as attrs};

use crate::live::{HttpResponseMatch, MatchedHttpRequest};
use crate::llm_pipeline::projection::ProjectionBatch as ActionBatch;

use super::payload_sequence_start;
use super::{BindingOwnershipIndex, IndexedQueue, StreamOwnershipIndex};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::llm_pipeline) struct LlmStreamKey {
    pub(in crate::llm_pipeline) trace_id: TraceId,
    pub(in crate::llm_pipeline) process: ProcessIdentity,
    pub(in crate::llm_pipeline) stream_key: String,
    pub(in crate::llm_pipeline) http_stream_id: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CorrelationStreamOwner {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
}

impl CorrelationStreamOwner {
    fn from_stream_key(key: &LlmStreamKey) -> Self {
        Self {
            trace_id: key.trace_id,
            process: key.process,
            stream_key: key.stream_key.clone(),
        }
    }

    fn from_identity(identity: &PayloadStreamIdentity) -> Self {
        Self {
            trace_id: identity.trace_id,
            process: identity.process,
            stream_key: identity.stream_key.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::llm_pipeline) struct OpenLlmRequest {
    pub(in crate::llm_pipeline) action: SemanticAction,
    pub(in crate::llm_pipeline) sequence_start: u64,
    pub(in crate::llm_pipeline) sequence_end: u64,
}

/// Ordering key for pairing open requests with responses on streams that have
/// no confirmed HTTP exchange (pure TLS tunnels): (start_time, sequence_start).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::llm_pipeline) struct LlmActionOrder {
    pub(in crate::llm_pipeline) start_time: SystemTime,
    pub(in crate::llm_pipeline) sequence_start: u64,
}

impl LlmActionOrder {
    pub(in crate::llm_pipeline) fn from_action(action: &SemanticAction) -> Option<Self> {
        Some(Self {
            start_time: action.start_time,
            sequence_start: payload_sequence_start(action)?,
        })
    }
}

pub(in crate::llm_pipeline) struct PendingLlmResponse {
    pub(in crate::llm_pipeline) action: SemanticAction,
    pub(in crate::llm_pipeline) provider_response_id: Option<String>,
    pub(in crate::llm_pipeline) closed: bool,
    pub(in crate::llm_pipeline) compacted: bool,
}

pub(in crate::llm_pipeline) struct ClosedPendingResponseMarker {
    pub(in crate::llm_pipeline) stream_key: LlmStreamKey,
    pub(in crate::llm_pipeline) response_action_id: String,
}

pub(in crate::llm_pipeline) struct IncompleteHttp1Response {
    pub(in crate::llm_pipeline) sequence: u64,
    pub(in crate::llm_pipeline) request: Option<MatchedHttpRequest>,
}

pub(in crate::llm_pipeline) struct DamagedHttpResponseBinding {
    pub(in crate::llm_pipeline) stream_key: LlmStreamKey,
    pub(in crate::llm_pipeline) http_response: SemanticAction,
}

pub(in crate::llm_pipeline) struct LateHttpFailureBinding {
    pub(in crate::llm_pipeline) stream_key: LlmStreamKey,
    pub(in crate::llm_pipeline) request: SemanticAction,
}

pub(in crate::llm_pipeline) struct ClosedLlmExchangeBinding {
    pub(in crate::llm_pipeline) request: OpenLlmRequest,
    pub(in crate::llm_pipeline) response: PendingLlmResponse,
}

#[derive(Clone)]
pub(in crate::llm_pipeline) struct ActiveLlmResponseBinding {
    pub(in crate::llm_pipeline) request: SemanticAction,
    pub(in crate::llm_pipeline) response: SemanticAction,
    pub(in crate::llm_pipeline) http_request_action_id: String,
    pub(in crate::llm_pipeline) http_response_action_id: String,
}

pub(in crate::llm_pipeline) struct CorrelationCoordinator {
    pub(in crate::llm_pipeline) open_requests: HashMap<LlmStreamKey, IndexedQueue<OpenLlmRequest>>,
    pub(in crate::llm_pipeline) pending_responses:
        HashMap<LlmStreamKey, IndexedQueue<PendingLlmResponse>>,
    pub(in crate::llm_pipeline) confirmed_http_exchanges:
        HashMap<LlmStreamKey, IndexedQueue<HttpResponseMatch>>,
    pub(in crate::llm_pipeline) closed_llm_exchanges:
        HashMap<LlmStreamKey, IndexedQueue<ClosedLlmExchangeBinding>>,
    closed_pending_responses:
        HashMap<CorrelationStreamOwner, IndexedQueue<ClosedPendingResponseMarker>>,
    closed_pending_response_ids_by_stream: HashMap<LlmStreamKey, HashSet<String>>,
    pub(in crate::llm_pipeline) incomplete_http1_responses:
        BTreeMap<LlmStreamKey, IncompleteHttp1Response>,
    pub(in crate::llm_pipeline) damaged_http_responses:
        BTreeMap<LlmStreamKey, VecDeque<SemanticAction>>,
    pub(in crate::llm_pipeline) damaged_response_bindings:
        BTreeMap<(TraceId, String), DamagedHttpResponseBinding>,
    pub(in crate::llm_pipeline) late_http_failure_bindings:
        BTreeMap<(TraceId, String), LateHttpFailureBinding>,
    pub(in crate::llm_pipeline) localized_http1_request_outputs:
        BTreeMap<LlmStreamKey, ActionBatch>,
    pub(in crate::llm_pipeline) active_response_requests:
        HashMap<(TraceId, String), ActiveLlmResponseBinding>,
    pub(in crate::llm_pipeline) damaged_binding_owners: BindingOwnershipIndex,
    pub(in crate::llm_pipeline) late_failure_binding_owners: BindingOwnershipIndex,
    pub(in crate::llm_pipeline) active_binding_owners: BindingOwnershipIndex,
    pub(in crate::llm_pipeline) stream_owners: StreamOwnershipIndex,
    pub(in crate::llm_pipeline) max_confirmed_http_exchanges_per_stream: usize,
    pub(in crate::llm_pipeline) max_pending_requests_per_stream: usize,
    pub(in crate::llm_pipeline) max_pending_responses_per_stream: usize,
}

impl CorrelationCoordinator {
    pub(in crate::llm_pipeline) fn new(
        max_confirmed_http_exchanges_per_stream: usize,
        state: LlmProjectionStateConfig,
    ) -> Self {
        Self {
            open_requests: HashMap::new(),
            pending_responses: HashMap::new(),
            confirmed_http_exchanges: HashMap::new(),
            closed_llm_exchanges: HashMap::new(),
            closed_pending_responses: HashMap::new(),
            closed_pending_response_ids_by_stream: HashMap::new(),
            incomplete_http1_responses: BTreeMap::new(),
            damaged_http_responses: BTreeMap::new(),
            damaged_response_bindings: BTreeMap::new(),
            late_http_failure_bindings: BTreeMap::new(),
            localized_http1_request_outputs: BTreeMap::new(),
            active_response_requests: HashMap::new(),
            damaged_binding_owners: BindingOwnershipIndex::new(validated_limit(
                state.max_damaged_response_bindings_per_trace,
            )),
            late_failure_binding_owners: BindingOwnershipIndex::new(validated_limit(
                state.max_damaged_response_bindings_per_trace,
            )),
            active_binding_owners: BindingOwnershipIndex::new(validated_limit(
                state.max_active_response_bindings_per_trace,
            )),
            stream_owners: StreamOwnershipIndex::new(validated_limit(
                state.max_correlation_streams_per_trace,
            )),
            max_confirmed_http_exchanges_per_stream,
            max_pending_requests_per_stream: validated_limit(state.max_pending_requests_per_stream),
            max_pending_responses_per_stream: validated_limit(
                state.max_pending_responses_per_stream,
            ),
        }
    }

    pub(in crate::llm_pipeline) fn open_requests_for_trace(
        &self,
        trace_id: TraceId,
    ) -> Vec<SemanticAction> {
        self.open_requests
            .iter()
            .filter(|(key, _)| key.trace_id == trace_id)
            .flat_map(|(_, requests)| requests.iter().map(|request| request.action.clone()))
            .collect()
    }

    pub(in crate::llm_pipeline) fn unconfirmed_streams_for_identity(
        &self,
        identity: &PayloadStreamIdentity,
    ) -> Vec<LlmStreamKey> {
        let stream_key = identity.stream_key.to_string();
        self.stream_owners.keys_for_stream_identity(
            identity.trace_id,
            identity.process,
            &stream_key,
        )
    }

    pub(in crate::llm_pipeline) fn mark_closed_pending_response(
        &mut self,
        stream_key: &LlmStreamKey,
        response_action_id: &str,
    ) {
        self.closed_pending_responses
            .entry(CorrelationStreamOwner::from_stream_key(stream_key))
            .or_default()
            .upsert(
                response_action_id.to_string(),
                ClosedPendingResponseMarker {
                    stream_key: stream_key.clone(),
                    response_action_id: response_action_id.to_string(),
                },
            );
        self.closed_pending_response_ids_by_stream
            .entry(stream_key.clone())
            .or_default()
            .insert(response_action_id.to_string());
    }

    pub(in crate::llm_pipeline) fn take_closed_pending_responses(
        &mut self,
        identity: &PayloadStreamIdentity,
    ) -> Vec<ClosedPendingResponseMarker> {
        let Some(markers) = self
            .closed_pending_responses
            .remove(&CorrelationStreamOwner::from_identity(identity))
        else {
            return Vec::new();
        };
        let markers = markers.into_values().collect::<Vec<_>>();
        for marker in &markers {
            self.remove_closed_pending_response_reverse(marker);
        }
        markers
    }

    pub(in crate::llm_pipeline) fn forget_closed_pending_stream(
        &mut self,
        stream_key: &LlmStreamKey,
    ) {
        let Some(response_ids) = self
            .closed_pending_response_ids_by_stream
            .remove(stream_key)
        else {
            return;
        };
        let owner = CorrelationStreamOwner::from_stream_key(stream_key);
        if let Some(markers) = self.closed_pending_responses.get_mut(&owner) {
            for response_id in response_ids {
                markers.remove(&response_id);
            }
            if markers.is_empty() {
                self.closed_pending_responses.remove(&owner);
            }
        }
    }

    pub(in crate::llm_pipeline) fn forget_closed_pending_trace(&mut self, trace_id: TraceId) {
        self.closed_pending_responses
            .retain(|owner, _| owner.trace_id != trace_id);
        self.closed_pending_response_ids_by_stream
            .retain(|stream_key, _| stream_key.trace_id != trace_id);
    }

    fn remove_closed_pending_response_reverse(&mut self, marker: &ClosedPendingResponseMarker) {
        let Some(response_ids) = self
            .closed_pending_response_ids_by_stream
            .get_mut(&marker.stream_key)
        else {
            return;
        };
        response_ids.remove(&marker.response_action_id);
        if response_ids.is_empty() {
            self.closed_pending_response_ids_by_stream
                .remove(&marker.stream_key);
        }
    }

    pub(in crate::llm_pipeline) fn compact_pending_response(
        &mut self,
        stream_key: &LlmStreamKey,
        response_action_id: &str,
    ) -> bool {
        let Some(response) = self
            .pending_responses
            .get_mut(stream_key)
            .and_then(|responses| responses.get_mut(response_action_id))
            .filter(|response| response.closed)
        else {
            return false;
        };
        response.compact_for_correlation();
        true
    }

    pub(in crate::llm_pipeline) fn forget_stream_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
    ) {
        let _ = self.take_closed_pending_responses(identity);
        let stream_key = identity.stream_key.to_string();
        let owned_keys = self.stream_owners.keys_for_stream_identity(
            identity.trace_id,
            identity.process,
            &stream_key,
        );
        for key in owned_keys {
            self.stream_owners.remove(&key);
            self.open_requests.remove(&key);
            self.pending_responses.remove(&key);
            self.confirmed_http_exchanges.remove(&key);
            self.closed_llm_exchanges.remove(&key);
            self.incomplete_http1_responses.remove(&key);
            self.damaged_http_responses.remove(&key);
            self.localized_http1_request_outputs.remove(&key);
        }
        let damaged_binding_keys = self.damaged_binding_owners.take_owner(
            identity.trace_id,
            identity.process,
            &stream_key,
        );
        for key in damaged_binding_keys {
            self.damaged_response_bindings.remove(&key);
        }
        let late_failure_binding_keys = self.late_failure_binding_owners.take_owner(
            identity.trace_id,
            identity.process,
            &stream_key,
        );
        for key in late_failure_binding_keys {
            self.late_http_failure_bindings.remove(&key);
        }
        let active_binding_keys =
            self.active_binding_owners
                .take_owner(identity.trace_id, identity.process, &stream_key);
        for key in active_binding_keys {
            self.active_response_requests.remove(&key);
        }
    }
}

impl LateHttpFailureBinding {
    pub(in crate::llm_pipeline) fn new(stream_key: LlmStreamKey, request: &SemanticAction) -> Self {
        Self {
            stream_key,
            request: compact_action(
                request,
                &[
                    attrs::llm_request::MODEL,
                    attrs::url::SCHEME,
                    attrs::url::PATH,
                    attrs::server::ADDRESS,
                    attrs::payload::STREAM_KEY,
                    attrs::payload::OPERATION_ID,
                    attrs::http_request::STREAM_ID,
                ],
            ),
        }
    }
}

impl ClosedLlmExchangeBinding {
    pub(in crate::llm_pipeline) fn new(
        request: OpenLlmRequest,
        mut response: PendingLlmResponse,
    ) -> Self {
        response.compact_for_correlation();
        response.provider_response_id = None;
        Self {
            request: OpenLlmRequest {
                action: compact_action(
                    &request.action,
                    &[
                        attrs::llm_request::MODEL,
                        attrs::http_request::METHOD,
                        attrs::url::SCHEME,
                        attrs::url::PATH,
                        attrs::server::ADDRESS,
                        attrs::payload::STREAM_KEY,
                        attrs::payload::OPERATION_ID,
                        attrs::payload::SEQUENCE,
                        attrs::payload::SEQUENCE_START,
                        attrs::payload::SEQUENCE_END,
                        attrs::http_request::STREAM_ID,
                    ],
                ),
                sequence_start: request.sequence_start,
                sequence_end: request.sequence_end,
            },
            response,
        }
    }

    pub(in crate::llm_pipeline) fn matches_http_exchange(
        &self,
        exchange: &HttpResponseMatch,
    ) -> bool {
        self.request.matches_http_request(&exchange.request)
            && self.response.matches_http_response(&exchange.response)
    }

    pub(in crate::llm_pipeline) fn precedes_http_exchange(
        &self,
        exchange: &HttpResponseMatch,
    ) -> bool {
        let Some(binding_sequence_end) = super::payload_sequence_end(&self.response.action) else {
            return false;
        };
        let Some(exchange_sequence) = exchange
            .response
            .attributes
            .get("payload_sequence")
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return false;
        };
        binding_sequence_end < exchange_sequence
    }
}

fn compact_action(action: &SemanticAction, retained_attributes: &[&str]) -> SemanticAction {
    let attributes = retained_attributes
        .iter()
        .filter_map(|key| {
            action
                .attributes
                .get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    SemanticAction {
        action_id: action.action_id.clone(),
        trace_id: action.trace_id,
        kind: action.kind,
        title: action.title.clone(),
        start_time: action.start_time,
        end_time: action.end_time,
        process: action.process,
        status: action.status,
        completeness: action.completeness,
        attributes,
        evidence: action.evidence.clone(),
    }
}

fn validated_limit(value: u32) -> usize {
    usize::try_from(value).expect("validated LLM projection-state limit must fit usize")
}

impl OpenLlmRequest {
    pub(in crate::llm_pipeline) fn matches_http_request(
        &self,
        request: &MatchedHttpRequest,
    ) -> bool {
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
    pub(in crate::llm_pipeline) fn compact_for_correlation(&mut self) {
        if self.compacted {
            return;
        }
        self.action = compact_action(
            &self.action,
            &[
                attrs::llm_response::MODEL,
                attrs::http_response::STATUS_CODE,
                attrs::http_response::REASON,
                attrs::http_response::STREAM_ID,
                attrs::payload::STREAM_KEY,
                attrs::payload::OPERATION_ID,
                attrs::payload::SEQUENCE,
                attrs::payload::SEQUENCE_START,
                attrs::payload::SEQUENCE_END,
            ],
        );
        self.compacted = true;
    }

    pub(in crate::llm_pipeline) fn matches_http_response(&self, response: &SemanticAction) -> bool {
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
        payload_sequence_start(&self.action)
            .zip(super::payload_sequence_end(&self.action))
            .is_some_and(|(start, end)| start <= sequence && sequence <= end)
    }
}

impl LlmStreamKey {
    pub(in crate::llm_pipeline) fn from_llm_request(action: &SemanticAction) -> Option<Self> {
        Self::from_llm_action(action, attrs::http_request::STREAM_ID)
    }

    pub(in crate::llm_pipeline) fn from_llm_response(action: &SemanticAction) -> Option<Self> {
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

    pub(in crate::llm_pipeline) fn from_http_response_candidates(
        action: &SemanticAction,
    ) -> Vec<Self> {
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

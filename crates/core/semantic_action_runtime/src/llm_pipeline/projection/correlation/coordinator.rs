//! Owned correlation state for LLM requests, responses, and HTTP exchanges.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

use config_core::daemon::LlmProjectionStateConfig;
use model_core::ids::TraceId;
use model_core::payload::PayloadStreamIdentity;
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, attr_keys as attrs};

use crate::live::{HttpResponseMatch, MatchedHttpRequest};
use crate::llm_pipeline::projection::ProjectionBatch as ActionBatch;

use super::payload_sequence_start;
use super::{BindingOwnershipIndex, StreamOwnershipIndex};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::llm_pipeline) struct LlmStreamKey {
    pub(in crate::llm_pipeline) trace_id: TraceId,
    pub(in crate::llm_pipeline) process: ProcessIdentity,
    pub(in crate::llm_pipeline) stream_key: String,
    pub(in crate::llm_pipeline) http_stream_id: Option<String>,
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
    pub(in crate::llm_pipeline) failed_response: SemanticAction,
}

pub(in crate::llm_pipeline) struct ActiveLlmResponseBinding {
    pub(in crate::llm_pipeline) request: SemanticAction,
    pub(in crate::llm_pipeline) http_request_action_id: String,
    pub(in crate::llm_pipeline) http_response_action_id: String,
}

pub(in crate::llm_pipeline) struct CorrelationCoordinator {
    pub(in crate::llm_pipeline) open_requests: BTreeMap<LlmStreamKey, VecDeque<OpenLlmRequest>>,
    pub(in crate::llm_pipeline) pending_responses:
        BTreeMap<LlmStreamKey, VecDeque<PendingLlmResponse>>,
    pub(in crate::llm_pipeline) confirmed_http_exchanges:
        BTreeMap<LlmStreamKey, VecDeque<HttpResponseMatch>>,
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
        BTreeMap<(TraceId, String), ActiveLlmResponseBinding>,
    pub(in crate::llm_pipeline) damaged_binding_owners: BindingOwnershipIndex,
    pub(in crate::llm_pipeline) late_failure_binding_owners: BindingOwnershipIndex,
    pub(in crate::llm_pipeline) active_binding_owners: BindingOwnershipIndex,
    pub(in crate::llm_pipeline) stream_owners: StreamOwnershipIndex,
    /// Streams for which a non-CONNECT confirmed HTTP exchange was observed.
    /// Pure TLS tunnels only ever produce a CONNECT exchange, so their LLM
    /// request/response pairing is not covered by the confirmed-exchange model
    /// and is reconciled at trace close.
    pub(in crate::llm_pipeline) http_exchange_streams: BTreeSet<LlmStreamKey>,
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
            open_requests: BTreeMap::new(),
            pending_responses: BTreeMap::new(),
            confirmed_http_exchanges: BTreeMap::new(),
            incomplete_http1_responses: BTreeMap::new(),
            damaged_http_responses: BTreeMap::new(),
            damaged_response_bindings: BTreeMap::new(),
            late_http_failure_bindings: BTreeMap::new(),
            localized_http1_request_outputs: BTreeMap::new(),
            active_response_requests: BTreeMap::new(),
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
            http_exchange_streams: BTreeSet::new(),
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

    pub(in crate::llm_pipeline) fn forget_stream_identity(
        &mut self,
        identity: &PayloadStreamIdentity,
    ) {
        let stream_key = identity.stream_key.to_string();
        let mut owned_keys = BTreeSet::new();
        Self::collect_owned_map_keys(&self.open_requests, identity, &stream_key, &mut owned_keys);
        Self::collect_owned_map_keys(
            &self.pending_responses,
            identity,
            &stream_key,
            &mut owned_keys,
        );
        Self::collect_owned_map_keys(
            &self.confirmed_http_exchanges,
            identity,
            &stream_key,
            &mut owned_keys,
        );
        Self::collect_owned_map_keys(
            &self.incomplete_http1_responses,
            identity,
            &stream_key,
            &mut owned_keys,
        );
        Self::collect_owned_map_keys(
            &self.damaged_http_responses,
            identity,
            &stream_key,
            &mut owned_keys,
        );
        Self::collect_owned_map_keys(
            &self.localized_http1_request_outputs,
            identity,
            &stream_key,
            &mut owned_keys,
        );
        let lower = Self::identity_lower_bound(identity, &stream_key);
        owned_keys.extend(
            self.http_exchange_streams
                .range(lower..)
                .take_while(|key| Self::matches_identity(key, identity, &stream_key))
                .cloned(),
        );
        for key in owned_keys {
            self.stream_owners.remove(&key);
            self.open_requests.remove(&key);
            self.pending_responses.remove(&key);
            self.confirmed_http_exchanges.remove(&key);
            self.http_exchange_streams.remove(&key);
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

    fn collect_owned_map_keys<V>(
        map: &BTreeMap<LlmStreamKey, V>,
        identity: &PayloadStreamIdentity,
        stream_key: &str,
        output: &mut BTreeSet<LlmStreamKey>,
    ) {
        let lower = Self::identity_lower_bound(identity, stream_key);
        output.extend(
            map.range(lower..)
                .take_while(|(key, _)| Self::matches_identity(key, identity, stream_key))
                .map(|(key, _)| key.clone()),
        );
    }

    fn identity_lower_bound(identity: &PayloadStreamIdentity, stream_key: &str) -> LlmStreamKey {
        LlmStreamKey {
            trace_id: identity.trace_id,
            process: identity.process,
            stream_key: stream_key.to_string(),
            http_stream_id: None,
        }
    }

    fn matches_identity(
        key: &LlmStreamKey,
        identity: &PayloadStreamIdentity,
        stream_key: &str,
    ) -> bool {
        key.trace_id == identity.trace_id
            && key.process == identity.process
            && key.stream_key == stream_key
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

//! Bounded HTTP request/response identity reconciliation.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use config_core::daemon::HttpExchangeConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticEvidence, attr_keys as attrs};

const HTTP_DIRECTION_ATTR: &str = "direction";
const HTTP_PAYLOAD_SEQUENCE_ATTR: &str = "payload_sequence";
const HTTP_STATUS_CODE_ATTR: &str = "status_code";
const HTTP_STREAM_ID_ATTR: &str = "stream_id";
const HTTP_STREAM_KEY_ATTR: &str = "stream_key";

pub(super) struct HttpExchangeTracker {
    streams: BTreeMap<HttpExchangeKey, HttpStreamState>,
    config: HttpExchangeConfig,
}

pub(super) struct HttpExchangeObservation {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) matches: Vec<HttpResponseMatch>,
}

#[derive(Clone, Debug)]
pub(super) struct HttpResponseMatch {
    pub(super) response: SemanticAction,
    pub(super) request: MatchedHttpRequest,
}

#[derive(Clone, Debug)]
pub(super) struct MatchedHttpRequest {
    pub(super) action_id: String,
    pub(super) evidence: Vec<SemanticEvidence>,
    pub(super) sequence: u64,
    pub(super) method: Option<String>,
    pub(super) target: Option<String>,
    pub(super) stream_id: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct HttpExchangeKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
    stream_id: Option<String>,
}

#[derive(Default)]
struct HttpStreamState {
    requests: VecDeque<PendingHttpRequest>,
    responses: VecDeque<PendingHttpResponse>,
    quarantined: bool,
}

#[derive(Clone, Debug)]
struct PendingHttpRequest {
    action: SemanticAction,
    sequence: u64,
    method: Option<String>,
    target: Option<String>,
    stream_id: Option<String>,
}

struct PendingHttpResponse {
    action: SemanticAction,
    status_code: u16,
    received_at: SystemTime,
    emitted_unassociated: bool,
}

impl HttpExchangeTracker {
    pub(super) fn new(config: HttpExchangeConfig) -> Self {
        Self {
            streams: BTreeMap::new(),
            config,
        }
    }

    pub(super) fn observe_http_message(
        &mut self,
        action: SemanticAction,
    ) -> HttpExchangeObservation {
        let Some(key) = HttpExchangeKey::from_http_message(&action) else {
            return HttpExchangeObservation::single(action);
        };
        let now = SystemTime::now();
        let state = self.streams.entry(key.clone()).or_default();
        state.expire_responses(&key, now, self.config.response_lateness);
        let observation = match http_message_direction_operation(&action) {
            Some(("outbound", "request")) => state.observe_request(
                &key,
                action,
                self.config.max_pending_requests_per_stream as usize,
            ),
            Some(("inbound", "response")) => state.observe_response(
                &key,
                action,
                now,
                self.config.max_pending_responses_per_stream as usize,
            ),
            _ => HttpExchangeObservation::single(action),
        };
        if state.is_empty() {
            self.streams.remove(&key);
        }
        observation
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id);
    }

    pub(super) fn quarantine_payload_stream(&mut self, segment: &PayloadSegment) {
        let stream_key = segment.stream_key.to_string();
        for (key, state) in &mut self.streams {
            if key.trace_id == segment.trace_id
                && key.process == segment.process
                && key.stream_key == stream_key
            {
                state.quarantine(key, "payload_discontinuity");
            }
        }
    }

    pub(super) fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        let stream_key = identity.stream_key.to_string();
        self.streams.retain(|key, _| {
            key.trace_id != identity.trace_id
                || key.process != identity.process
                || key.stream_key != stream_key
        });
    }
}

impl HttpExchangeObservation {
    fn single(action: SemanticAction) -> Self {
        Self {
            actions: vec![action],
            matches: Vec::new(),
        }
    }
}

impl HttpStreamState {
    fn observe_request(
        &mut self,
        key: &HttpExchangeKey,
        action: SemanticAction,
        max_pending: usize,
    ) -> HttpExchangeObservation {
        let mut observation = HttpExchangeObservation::single(action.clone());
        if self.quarantined {
            return observation;
        }
        let Some(sequence) = http_payload_sequence(&action) else {
            return observation;
        };
        if self.requests.len() >= max_pending {
            self.quarantine(key, "pending_request_capacity");
            return observation;
        }
        self.requests.push_back(PendingHttpRequest {
            sequence,
            method: action.attributes.get("method").cloned(),
            target: action.attributes.get("target").cloned(),
            stream_id: action.attributes.get(HTTP_STREAM_ID_ATTR).cloned(),
            action,
        });
        self.reconcile(key, &mut observation);
        observation
    }

    fn observe_response(
        &mut self,
        key: &HttpExchangeKey,
        action: SemanticAction,
        now: SystemTime,
        max_pending: usize,
    ) -> HttpExchangeObservation {
        if self.quarantined {
            return HttpExchangeObservation::single(action);
        }
        let Some(status_code) = http_status_code(&action) else {
            return HttpExchangeObservation::single(action);
        };
        if http_payload_sequence(&action).is_none() {
            return HttpExchangeObservation::single(action);
        }
        if self.responses.len() >= max_pending {
            self.quarantine(key, "pending_response_capacity");
            return HttpExchangeObservation::single(action);
        }
        let action_id = action.action_id.clone();
        self.responses.push_back(PendingHttpResponse {
            action: action.clone(),
            status_code,
            received_at: now,
            emitted_unassociated: false,
        });
        let mut observation = HttpExchangeObservation {
            actions: Vec::new(),
            matches: Vec::new(),
        };
        self.reconcile(key, &mut observation);
        if !observation
            .actions
            .iter()
            .any(|candidate| candidate.action_id == action_id)
        {
            if let Some(response) = self
                .responses
                .iter_mut()
                .find(|response| response.action.action_id == action_id)
            {
                response.emitted_unassociated = true;
            }
            observation.actions.push(action);
        }
        observation
    }

    fn reconcile(&mut self, key: &HttpExchangeKey, observation: &mut HttpExchangeObservation) {
        while let (Some(request), Some(response)) = (self.requests.front(), self.responses.front())
        {
            if request.action.start_time > response.action.start_time {
                self.responses.pop_front();
                tracing::warn!(
                    trace_id = key.trace_id.get(),
                    process_id = key.process.get(),
                    stream_key = %key.stream_key,
                    stream_id = ?key.stream_id,
                    reason = "response_precedes_request_start",
                    "kept causally impossible HTTP response unassociated"
                );
                continue;
            }
            let request = request.clone();
            let Some(mut response) = self.responses.pop_front() else {
                break;
            };
            if response.emitted_unassociated {
                response.action.attributes.insert(
                    "http.exchange.reconciled_late".to_string(),
                    "true".to_string(),
                );
                tracing::info!(
                    trace_id = key.trace_id.get(),
                    process_id = key.process.get(),
                    stream_key = %key.stream_key,
                    stream_id = ?key.stream_id,
                    request_action_id = %request.action.action_id,
                    response_action_id = %response.action.action_id,
                    "reconciled HTTP response projected before its request action"
                );
            }
            response.action.attributes.insert(
                attrs::http_response::REQUEST_ACTION_ID.to_string(),
                request.action.action_id.clone(),
            );
            let matched_request = MatchedHttpRequest {
                action_id: request.action.action_id,
                evidence: request.action.evidence,
                sequence: request.sequence,
                method: request.method,
                target: request.target,
                stream_id: request.stream_id,
            };
            if final_http_response(response.status_code) {
                self.requests.pop_front();
            }
            observation.actions.push(response.action.clone());
            observation.matches.push(HttpResponseMatch {
                response: response.action,
                request: matched_request,
            });
        }
    }

    fn expire_responses(
        &mut self,
        key: &HttpExchangeKey,
        now: SystemTime,
        response_lateness: std::time::Duration,
    ) {
        let expired = self.responses.front().is_some_and(|response| {
            now.duration_since(response.received_at)
                .is_ok_and(|age| age > response_lateness)
        });
        if expired {
            self.quarantine(key, "pending_response_expired");
        }
    }

    fn quarantine(&mut self, key: &HttpExchangeKey, reason: &'static str) {
        self.requests.clear();
        self.responses.clear();
        self.quarantined = true;
        tracing::warn!(
            trace_id = key.trace_id.get(),
            process_id = key.process.get(),
            stream_key = %key.stream_key,
            stream_id = ?key.stream_id,
            reason,
            "quarantined HTTP exchange identity tracking"
        );
    }

    fn is_empty(&self) -> bool {
        !self.quarantined && self.requests.is_empty() && self.responses.is_empty()
    }
}

impl HttpExchangeKey {
    fn from_http_message(action: &SemanticAction) -> Option<Self> {
        if action.kind != SemanticActionKind::HttpMessage {
            return None;
        }
        Some(Self {
            trace_id: action.trace_id,
            process: action.process.clone(),
            stream_key: action.attributes.get(HTTP_STREAM_KEY_ATTR)?.clone(),
            stream_id: action.attributes.get(HTTP_STREAM_ID_ATTR).cloned(),
        })
    }
}

fn http_message_direction_operation(action: &SemanticAction) -> Option<(&str, &str)> {
    Some((
        action.attributes.get(HTTP_DIRECTION_ATTR)?.as_str(),
        action.attributes.get(attrs::http::OPERATION)?.as_str(),
    ))
}

fn http_payload_sequence(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(HTTP_PAYLOAD_SEQUENCE_ATTR)?
        .parse()
        .ok()
}

fn http_status_code(action: &SemanticAction) -> Option<u16> {
    action.attributes.get(HTTP_STATUS_CODE_ATTR)?.parse().ok()
}

fn final_http_response(status_code: u16) -> bool {
    !(100..=199).contains(&status_code) || status_code == 101
}

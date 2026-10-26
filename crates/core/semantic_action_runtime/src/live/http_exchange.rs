//! Bounded HTTP request/response identity reconciliation.

mod stream;

use std::collections::{HashMap, VecDeque};
use std::time::SystemTime;

use config_core::daemon::HttpExchangeConfig;
use model_core::ids::TraceId;
use model_core::payload::{PayloadSourceBoundary, PayloadStreamIdentity};
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticEvidence, attr_keys as attrs};

const HTTP_DIRECTION_ATTR: &str = "direction";
const HTTP_PAYLOAD_SEQUENCE_ATTR: &str = "payload_sequence";
const HTTP_STATUS_CODE_ATTR: &str = "status_code";
const HTTP_STREAM_ID_ATTR: &str = "stream_id";

pub(crate) struct HttpExchangeTracker {
    streams: HashMap<HttpExchangeKey, HttpStreamState>,
    config: HttpExchangeConfig,
}

pub(crate) struct HttpExchangeObservation {
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) matches: Vec<HttpResponseMatch>,
}

#[derive(Clone, Debug)]
pub(crate) struct HttpResponseMatch {
    pub(crate) response: SemanticAction,
    pub(crate) request: MatchedHttpRequest,
    pub(crate) status_code: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct MatchedHttpRequest {
    pub(crate) action_id: String,
    pub(crate) evidence: Vec<SemanticEvidence>,
    pub(crate) method: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct HttpExchangeKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    source_boundary: PayloadSourceBoundary,
    stream_key: String,
    stream_id: Option<String>,
}

#[derive(Default)]
struct HttpStreamState {
    requests: VecDeque<PendingHttpRequest>,
    responses: VecDeque<PendingHttpResponse>,
    quarantine: Option<HttpStreamQuarantine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpStreamQuarantine {
    reason: HttpStreamQuarantineReason,
    response_watermark: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HttpStreamQuarantineReason {
    PendingRequestCapacity,
    PendingResponseCapacity,
    PendingResponseExpired,
}

impl HttpStreamQuarantineReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PendingRequestCapacity => "pending_request_capacity",
            Self::PendingResponseCapacity => "pending_response_capacity",
            Self::PendingResponseExpired => "pending_response_expired",
        }
    }

    const fn is_recoverable_at_request_boundary(self) -> bool {
        matches!(self, Self::PendingResponseExpired)
    }
}

impl HttpStreamQuarantine {
    const fn new(
        reason: HttpStreamQuarantineReason,
        response_watermark: Option<SystemTime>,
    ) -> Self {
        Self {
            reason,
            response_watermark,
        }
    }

    fn allows_request_resynchronization(&self, request_started_at: SystemTime) -> bool {
        self.reason.is_recoverable_at_request_boundary()
            && self
                .response_watermark
                .is_none_or(|watermark| request_started_at > watermark)
    }

    fn observe_unassociated_response(&mut self, response_started_at: SystemTime) {
        if !self.reason.is_recoverable_at_request_boundary() {
            return;
        }
        if self
            .response_watermark
            .is_none_or(|watermark| response_started_at > watermark)
        {
            self.response_watermark = Some(response_started_at);
        }
    }
}

#[derive(Clone, Debug)]
struct PendingHttpRequest {
    action_id: String,
    start_time: SystemTime,
    evidence: Vec<SemanticEvidence>,
    method: Option<String>,
}

struct PendingHttpResponse {
    action: SemanticAction,
    status_code: u16,
    observed_at: SystemTime,
    emitted_unassociated: bool,
}

impl HttpExchangeTracker {
    pub(crate) fn new(config: HttpExchangeConfig) -> Self {
        Self {
            streams: HashMap::new(),
            config,
        }
    }

    pub(crate) fn observe_http_message(
        &mut self,
        action: SemanticAction,
        identity: &PayloadStreamIdentity,
    ) -> HttpExchangeObservation {
        let Some(key) = HttpExchangeKey::from_http_message(&action, identity) else {
            return HttpExchangeObservation::single(action);
        };
        let observed_at = action.start_time;
        let state = self.streams.entry(key.clone()).or_default();
        let operation = http_message_direction_operation(&action);
        if operation == Some(("inbound", "response")) {
            state.expire_responses(&key, observed_at, self.config.response_lateness);
        }
        let observation = match operation {
            Some(("outbound", "request")) => {
                let observation = state.observe_request(
                    &key,
                    action,
                    self.config.max_pending_requests_per_stream as usize,
                );
                if observation.matches.is_empty() {
                    state.expire_responses(&key, observed_at, self.config.response_lateness);
                }
                observation
            }
            Some(("inbound", "response")) => state.observe_response(
                &key,
                action,
                observed_at,
                self.config.max_pending_responses_per_stream as usize,
            ),
            _ => HttpExchangeObservation::single(action),
        };
        if state.is_empty() {
            self.streams.remove(&key);
        }
        observation
    }

    pub(crate) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id);
    }

    pub(crate) fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        let stream_key = identity.stream_key.to_string();
        self.streams.retain(|key, _| {
            key.trace_id != identity.trace_id
                || key.process != identity.process
                || key.source_boundary != identity.source_boundary
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

impl HttpExchangeKey {
    fn from_http_message(
        action: &SemanticAction,
        identity: &PayloadStreamIdentity,
    ) -> Option<Self> {
        if action.kind != SemanticActionKind::HttpMessage
            || action.trace_id != identity.trace_id
            || action.process != identity.process
        {
            return None;
        }
        Some(Self {
            trace_id: action.trace_id,
            process: action.process.clone(),
            source_boundary: identity.source_boundary,
            stream_key: identity.stream_key.to_string(),
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

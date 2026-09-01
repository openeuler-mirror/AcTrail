//! Bounded HTTP request/response identity reconciliation.

use std::collections::{HashMap, VecDeque};
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
    streams: HashMap<HttpExchangeKey, HttpStreamState>,
    config: HttpExchangeConfig,
}

pub(super) struct HttpExchangeObservation {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) matches: Vec<HttpResponseMatch>,
    pub(super) damaged_responses: Vec<SemanticAction>,
    pub(super) unmatched_responses: Vec<SemanticAction>,
}

pub(super) enum DamagedHttp1RequestOutcome {
    Tombstoned,
    MissingPending,
    Unsafe,
}

#[derive(Clone, Debug)]
pub(crate) struct HttpResponseMatch {
    pub(crate) response: SemanticAction,
    pub(crate) request: MatchedHttpRequest,
    pub(crate) status_code: u16,
    pub(crate) reconciled_late: bool,
}

impl HttpResponseMatch {
    pub(crate) fn eligible_for_llm_correlation(&self) -> bool {
        self.request.method.as_deref() != Some("CONNECT")
            && !(100..=199).contains(&self.status_code)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MatchedHttpRequest {
    pub(crate) action_id: String,
    pub(crate) evidence: Vec<SemanticEvidence>,
    pub(crate) sequence: u64,
    pub(crate) method: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) stream_id: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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
    quarantine: Option<HttpStreamQuarantine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpStreamQuarantine {
    reason: HttpStreamQuarantineReason,
    response_watermark: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HttpStreamQuarantineReason {
    PayloadDiscontinuity,
    PendingRequestCapacity,
    PendingResponseCapacity,
    PendingResponseExpired,
}

impl HttpStreamQuarantineReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PayloadDiscontinuity => "payload_discontinuity",
            Self::PendingRequestCapacity => "pending_request_capacity",
            Self::PendingResponseCapacity => "pending_response_capacity",
            Self::PendingResponseExpired => "pending_response_expired",
        }
    }

    const fn is_recoverable_at_request_boundary(self) -> bool {
        matches!(
            self,
            Self::PayloadDiscontinuity | Self::PendingResponseExpired
        )
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
enum PendingHttpRequest {
    Observed {
        action_id: String,
        start_time: SystemTime,
        evidence: Vec<SemanticEvidence>,
        sequence: u64,
        method: Option<String>,
        target: Option<String>,
        stream_id: Option<String>,
    },
    Damaged,
}

struct PendingHttpResponse {
    action: SemanticAction,
    status_code: u16,
    observed_at: SystemTime,
    emitted_unassociated: bool,
}

impl HttpExchangeTracker {
    pub(super) fn new(config: HttpExchangeConfig) -> Self {
        Self {
            streams: HashMap::new(),
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
                state.quarantine(key, HttpStreamQuarantineReason::PayloadDiscontinuity, None);
            }
        }
    }

    pub(super) fn observe_damaged_http1_request(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        header_projected: bool,
    ) -> DamagedHttp1RequestOutcome {
        let key = HttpExchangeKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            stream_id: None,
        };
        let state = self.streams.entry(key.clone()).or_default();
        state.observe_damaged_request(
            &key,
            self.config.max_pending_requests_per_stream as usize,
            sequence,
            header_projected,
        )
    }

    pub(super) fn observe_damaged_http1_response(
        &mut self,
        segment: &PayloadSegment,
    ) -> Option<MatchedHttpRequest> {
        let key = HttpExchangeKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            stream_id: None,
        };
        let state = self.streams.get_mut(&key)?;
        let request = state.observe_damaged_response();
        if state.is_empty() {
            self.streams.remove(&key);
        }
        request
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
            damaged_responses: Vec::new(),
            unmatched_responses: Vec::new(),
        }
    }
}

impl HttpStreamState {
    fn observe_damaged_response(&mut self) -> Option<MatchedHttpRequest> {
        if self.quarantine.is_some() {
            return None;
        }
        match self.requests.pop_front()? {
            PendingHttpRequest::Observed {
                action_id,
                evidence,
                sequence,
                method,
                target,
                stream_id,
                ..
            } => Some(MatchedHttpRequest {
                action_id,
                evidence,
                sequence,
                method,
                target,
                stream_id,
            }),
            PendingHttpRequest::Damaged => None,
        }
    }

    fn observe_request(
        &mut self,
        key: &HttpExchangeKey,
        action: SemanticAction,
        max_pending: usize,
    ) -> HttpExchangeObservation {
        let mut observation = HttpExchangeObservation::single(action.clone());
        let Some(sequence) = http_payload_sequence(&action) else {
            return observation;
        };
        if self.quarantine.is_some() {
            let quarantine = self.quarantine.as_ref().expect("quarantine was present");
            if !quarantine.allows_request_resynchronization(action.start_time) {
                return observation;
            }
            let reason = quarantine.reason;
            // The application parser emits a request only after recognizing a complete HTTP
            // request line and headers. After a payload gap, that is a trusted byte boundary for
            // abandoning the damaged exchange without poisoning the rest of a keep-alive stream.
            // The response watermark rejects a delayed request that could belong to an ignored
            // response; only a causally newer request may restart FIFO pairing.
            self.quarantine = None;
            tracing::info!(
                trace_id = key.trace_id.get(),
                process_id = key.process.get(),
                stream_key = %key.stream_key,
                stream_id = ?key.stream_id,
                request_action_id = %action.action_id,
                reason = reason.as_str(),
                "resynchronized HTTP exchange identity tracking at request boundary"
            );
        }
        if self.requests.len() >= max_pending {
            self.quarantine(
                key,
                HttpStreamQuarantineReason::PendingRequestCapacity,
                None,
            );
            return observation;
        }
        self.requests.push_back(PendingHttpRequest::Observed {
            action_id: action.action_id.clone(),
            start_time: action.start_time,
            evidence: action.evidence.clone(),
            sequence,
            method: action.attributes.get("method").cloned(),
            target: action.attributes.get("target").cloned(),
            stream_id: action.attributes.get(HTTP_STREAM_ID_ATTR).cloned(),
        });
        self.reconcile(key, &mut observation);
        observation
    }

    fn observe_damaged_request(
        &mut self,
        key: &HttpExchangeKey,
        max_pending: usize,
        message_sequence: u64,
        header_projected: bool,
    ) -> DamagedHttp1RequestOutcome {
        if self.quarantine.is_some() {
            return DamagedHttp1RequestOutcome::Unsafe;
        }
        let existing = header_projected.then(|| {
            self.requests.iter().rposition(|request| {
                matches!(
                    request,
                    PendingHttpRequest::Observed { sequence, .. } if *sequence == message_sequence
                )
            })
        });
        if header_projected && existing.flatten().is_none() {
            return DamagedHttp1RequestOutcome::MissingPending;
        }
        if let Some(index) = existing.flatten() {
            self.requests[index] = PendingHttpRequest::Damaged;
        } else {
            if self.requests.len() >= max_pending {
                self.quarantine(
                    key,
                    HttpStreamQuarantineReason::PendingRequestCapacity,
                    None,
                );
                return DamagedHttp1RequestOutcome::Unsafe;
            }
            self.requests.push_back(PendingHttpRequest::Damaged);
        }
        tracing::warn!(
            trace_id = key.trace_id.get(),
            process_id = key.process.get(),
            stream_key = %key.stream_key,
            message_sequence,
            "kept HTTP/1 request ordinal with a damaged-message tombstone"
        );
        let mut observation = HttpExchangeObservation {
            actions: Vec::new(),
            matches: Vec::new(),
            damaged_responses: Vec::new(),
            unmatched_responses: Vec::new(),
        };
        self.reconcile(key, &mut observation);
        DamagedHttp1RequestOutcome::Tombstoned
    }

    fn observe_response(
        &mut self,
        key: &HttpExchangeKey,
        action: SemanticAction,
        observed_at: SystemTime,
        max_pending: usize,
    ) -> HttpExchangeObservation {
        let Some(status_code) = http_status_code(&action) else {
            return HttpExchangeObservation::single(action);
        };
        if http_payload_sequence(&action).is_none() {
            return HttpExchangeObservation::single(action);
        }
        if let Some(quarantine) = self.quarantine.as_mut() {
            quarantine.observe_unassociated_response(action.start_time);
            return HttpExchangeObservation::single(action);
        }
        if self.responses.len() >= max_pending {
            self.quarantine(
                key,
                HttpStreamQuarantineReason::PendingResponseCapacity,
                None,
            );
            return HttpExchangeObservation::single(action);
        }
        let action_id = action.action_id.clone();
        self.responses.push_back(PendingHttpResponse {
            action: action.clone(),
            status_code,
            observed_at,
            emitted_unassociated: false,
        });
        let mut observation = HttpExchangeObservation {
            actions: Vec::new(),
            matches: Vec::new(),
            damaged_responses: Vec::new(),
            unmatched_responses: Vec::new(),
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
            observation.unmatched_responses.push(action.clone());
            observation.actions.push(action);
        }
        observation
    }

    fn reconcile(&mut self, key: &HttpExchangeKey, observation: &mut HttpExchangeObservation) {
        while let (Some(request), Some(response)) = (self.requests.front(), self.responses.front())
        {
            if matches!(request, PendingHttpRequest::Damaged) {
                let final_response = final_http_response(response.status_code);
                let Some(response) = self.responses.pop_front() else {
                    break;
                };
                observation.damaged_responses.push(response.action);
                if final_response {
                    self.requests.pop_front();
                }
                continue;
            }
            let PendingHttpRequest::Observed {
                action_id: request_action_id,
                start_time: request_start_time,
                evidence: request_evidence,
                sequence: request_sequence,
                method: request_method,
                target: request_target,
                stream_id: request_stream_id,
            } = request
            else {
                unreachable!();
            };
            if *request_start_time > response.action.start_time {
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
            let request_action_id = request_action_id.clone();
            let request_evidence = request_evidence.clone();
            let request_sequence = *request_sequence;
            let request_method = request_method.clone();
            let request_target = request_target.clone();
            let request_stream_id = request_stream_id.clone();
            let Some(response) = self.responses.pop_front() else {
                break;
            };
            if response.emitted_unassociated {
                tracing::info!(
                    trace_id = key.trace_id.get(),
                    process_id = key.process.get(),
                    stream_key = %key.stream_key,
                    stream_id = ?key.stream_id,
                    request_action_id = %request_action_id,
                    response_action_id = %response.action.action_id,
                    "reconciled HTTP response projected before its request action"
                );
            }
            let matched_request = MatchedHttpRequest {
                action_id: request_action_id,
                evidence: request_evidence,
                sequence: request_sequence,
                method: request_method,
                target: request_target,
                stream_id: request_stream_id,
            };
            if final_http_response(response.status_code) {
                self.requests.pop_front();
            }
            if !response.emitted_unassociated {
                observation.actions.push(response.action.clone());
            }
            observation.matches.push(HttpResponseMatch {
                response: response.action,
                request: matched_request,
                status_code: response.status_code,
                reconciled_late: response.emitted_unassociated,
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
            now.duration_since(response.observed_at)
                .is_ok_and(|age| age > response_lateness)
        });
        if expired {
            let response_watermark = self
                .responses
                .iter()
                .map(|response| response.action.start_time)
                .max();
            self.quarantine(
                key,
                HttpStreamQuarantineReason::PendingResponseExpired,
                response_watermark,
            );
        }
    }

    fn quarantine(
        &mut self,
        key: &HttpExchangeKey,
        reason: HttpStreamQuarantineReason,
        response_watermark: Option<SystemTime>,
    ) {
        self.requests.clear();
        self.responses.clear();
        self.quarantine = Some(HttpStreamQuarantine::new(reason, response_watermark));
        tracing::warn!(
            trace_id = key.trace_id.get(),
            process_id = key.process.get(),
            stream_key = %key.stream_key,
            stream_id = ?key.stream_id,
            reason = reason.as_str(),
            "quarantined HTTP exchange identity tracking"
        );
    }

    fn is_empty(&self) -> bool {
        self.quarantine.is_none() && self.requests.is_empty() && self.responses.is_empty()
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    use semantic_action::{SemanticActionCompleteness, SemanticActionStatus};

    use super::*;

    #[test]
    fn payload_gap_recovers_at_next_http_request_boundary() {
        let mut tracker = HttpExchangeTracker::new(HttpExchangeConfig::default());
        let first_request = http_request("request-1", 1, 1);
        let key = HttpExchangeKey::from_http_message(&first_request).unwrap();
        tracker.observe_http_message(first_request);
        tracker.streams.get_mut(&key).unwrap().quarantine(
            &key,
            HttpStreamQuarantineReason::PayloadDiscontinuity,
            None,
        );

        let unmatched = tracker.observe_http_message(http_response("response-1", 1, 2));
        assert!(unmatched.matches.is_empty());
        assert_eq!(unmatched.actions.len(), 1);
        assert!(
            !unmatched.actions[0]
                .attributes
                .contains_key(attrs::http_response::REQUEST_ACTION_ID)
        );

        tracker.observe_http_message(http_request("request-2", 4, 3));
        let recovered = tracker.observe_http_message(http_response("response-2", 229, 4));
        assert_eq!(recovered.matches.len(), 1);
        assert_eq!(recovered.matches[0].request.action_id, "request-2");
        assert_eq!(
            recovered.actions[0]
                .attributes
                .get(attrs::http_response::REQUEST_ACTION_ID)
                .map(String::as_str),
            Some("request-2")
        );
    }

    #[test]
    fn payload_gap_rejects_delayed_request_before_ignored_response() {
        let mut tracker = HttpExchangeTracker::new(HttpExchangeConfig::default());
        let first_request = http_request("request-1", 1, 1);
        let key = HttpExchangeKey::from_http_message(&first_request).unwrap();
        tracker.observe_http_message(first_request);
        tracker.streams.get_mut(&key).unwrap().quarantine(
            &key,
            HttpStreamQuarantineReason::PayloadDiscontinuity,
            None,
        );

        tracker.observe_http_message(http_response("response-ignored", 2, 5));
        tracker.observe_http_message(http_request("request-delayed", 3, 4));
        assert!(tracker.streams.get(&key).unwrap().quarantine.is_some());

        tracker.observe_http_message(http_request("request-new", 4, 6));
        let recovered = tracker.observe_http_message(http_response("response-new", 5, 7));
        assert_eq!(recovered.matches.len(), 1);
        assert_eq!(recovered.matches[0].request.action_id, "request-new");
    }

    #[test]
    fn expired_response_recovers_at_causally_newer_request_boundary() {
        let mut tracker = HttpExchangeTracker::new(HttpExchangeConfig::default());
        let orphan = http_response("response-orphan", 1, 2);
        let key = HttpExchangeKey::from_http_message(&orphan).unwrap();
        tracker.observe_http_message(orphan);
        let state = tracker.streams.get_mut(&key).unwrap();
        state.responses.front_mut().unwrap().received_at = UNIX_EPOCH;
        state.expire_responses(
            &key,
            UNIX_EPOCH + Duration::from_secs(10),
            Duration::from_secs(1),
        );
        assert_eq!(
            state.quarantine.as_ref().map(|state| state.reason),
            Some(HttpStreamQuarantineReason::PendingResponseExpired)
        );

        tracker.observe_http_message(http_request("request-delayed", 2, 1));
        assert!(tracker.streams.get(&key).unwrap().quarantine.is_some());

        tracker.observe_http_message(http_request("request-new", 3, 3));
        let recovered = tracker.observe_http_message(http_response("response-new", 4, 4));
        assert_eq!(recovered.matches.len(), 1);
        assert_eq!(recovered.matches[0].request.action_id, "request-new");
        assert_eq!(
            recovered.actions[0]
                .attributes
                .get(attrs::http_response::REQUEST_ACTION_ID)
                .map(String::as_str),
            Some("request-new")
        );
    }

    #[test]
    fn capacity_quarantine_does_not_resynchronize_at_request_boundary() {
        let mut tracker = HttpExchangeTracker::new(HttpExchangeConfig::default());
        let first_request = http_request("request-1", 1, 1);
        let key = HttpExchangeKey::from_http_message(&first_request).unwrap();
        tracker.observe_http_message(first_request);
        tracker.streams.get_mut(&key).unwrap().quarantine(
            &key,
            HttpStreamQuarantineReason::PendingRequestCapacity,
            None,
        );

        tracker.observe_http_message(http_request("request-2", 4, 2));
        let unmatched = tracker.observe_http_message(http_response("response-2", 229, 3));
        assert!(unmatched.matches.is_empty());
        assert!(tracker.streams.get(&key).unwrap().quarantine.is_some());
    }

    fn http_request(action_id: &str, sequence: u64, seconds: u64) -> SemanticAction {
        http_action(action_id, "outbound", "request", sequence, seconds, None)
    }

    fn http_response(action_id: &str, sequence: u64, seconds: u64) -> SemanticAction {
        http_action(
            action_id,
            "inbound",
            "response",
            sequence,
            seconds,
            Some(200),
        )
    }

    fn http_action(
        action_id: &str,
        direction: &str,
        operation: &str,
        sequence: u64,
        seconds: u64,
        status_code: Option<u16>,
    ) -> SemanticAction {
        let mut attributes = BTreeMap::from([
            (HTTP_DIRECTION_ATTR.to_string(), direction.to_string()),
            (attrs::http::OPERATION.to_string(), operation.to_string()),
            (HTTP_PAYLOAD_SEQUENCE_ATTR.to_string(), sequence.to_string()),
            (
                HTTP_STREAM_KEY_ATTR.to_string(),
                "socket:42:19:18".to_string(),
            ),
        ]);
        if let Some(status_code) = status_code {
            attributes.insert(HTTP_STATUS_CODE_ATTR.to_string(), status_code.to_string());
        }
        let start_time = UNIX_EPOCH + Duration::from_secs(seconds);
        SemanticAction {
            action_id: action_id.to_string(),
            trace_id: TraceId::new(2),
            kind: SemanticActionKind::HttpMessage,
            title: action_id.to_string(),
            start_time,
            end_time: Some(start_time),
            process: ProcessIdentity::new(14),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            attributes,
            evidence: Vec::new(),
        }
    }
}

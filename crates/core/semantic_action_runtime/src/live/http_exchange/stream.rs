//! Per-stream HTTP exchange queues and quarantine transitions.

use super::*;

impl HttpStreamState {
    pub(super) fn observe_request(
        &mut self,
        key: &HttpExchangeKey,
        action: SemanticAction,
        max_pending: usize,
    ) -> HttpExchangeObservation {
        let mut observation = HttpExchangeObservation::single(action.clone());
        let Some(_) = http_payload_sequence(&action) else {
            return observation;
        };
        if self.quarantine.is_some() {
            let quarantine = self.quarantine.as_ref().expect("quarantine was present");
            if !quarantine.allows_request_resynchronization(action.start_time) {
                return observation;
            }
            let reason = quarantine.reason;
            // The protocol owner emits a request at its confirmed head boundary.
            // An expired pending response can recover at a causally newer request.
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
        self.requests.push_back(PendingHttpRequest {
            action_id: action.action_id.clone(),
            start_time: action.start_time,
            evidence: action.evidence.clone(),
            method: action.attributes.get("method").cloned(),
        });
        self.reconcile(key, &mut observation);
        observation
    }

    pub(super) fn observe_response(
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
            let PendingHttpRequest {
                action_id: request_action_id,
                start_time: request_start_time,
                evidence: request_evidence,
                method: request_method,
            } = request;
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
            let request_method = request_method.clone();
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
                    request_action_id = %request_action_id,
                    response_action_id = %response.action.action_id,
                    "reconciled HTTP response projected before its request action"
                );
            }
            response.action.attributes.insert(
                attrs::http_response::REQUEST_ACTION_ID.to_string(),
                request_action_id.clone(),
            );
            let matched_request = MatchedHttpRequest {
                action_id: request_action_id,
                evidence: request_evidence,
                method: request_method,
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
            });
        }
    }

    pub(super) fn expire_responses(
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

    pub(super) fn quarantine(
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

    pub(super) fn is_empty(&self) -> bool {
        self.quarantine.is_none() && self.requests.is_empty() && self.responses.is_empty()
    }
}

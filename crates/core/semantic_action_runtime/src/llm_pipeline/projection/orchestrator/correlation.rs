//! HTTP and LLM request/response correlation orchestration.
use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};
use std::collections::BTreeSet;
use std::time::SystemTime;

use crate::live::{HttpResponseMatch, MatchedHttpRequest};
use crate::llm_pipeline::projection::correlation::{
    self as call, ActiveLlmResponseBinding, ClosedLlmExchangeBinding, IndexedQueue, LlmActionOrder,
    LlmStreamKey,
};
use crate::llm_pipeline::projection::projector::capacity_diagnostic;
use crate::llm_pipeline::transport::websocket;

use super::super::ProjectionBatch as LiveLlmOutput;
use super::super::links::{LlmHttpRequestLink, LlmHttpResponseLink};
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn remember_confirmed_http_exchange(
        &mut self,
        matched: HttpResponseMatch,
    ) -> (Option<LlmStreamKey>, LiveLlmOutput) {
        let mut output = LiveLlmOutput::default();
        if !matched.eligible_for_llm_correlation() {
            return (None, output);
        }
        let candidates = LlmStreamKey::from_http_response_candidates(&matched.response);
        let stream_key = candidates
            .iter()
            .find(|candidate| self.http_exchange_ready(candidate, &matched))
            .or_else(|| {
                candidates.iter().find(|candidate| {
                    self.correlation.open_requests.contains_key(*candidate)
                        && self.correlation.pending_responses.contains_key(*candidate)
                })
            })
            .or_else(|| {
                candidates.iter().find(|candidate| {
                    self.correlation.open_requests.contains_key(*candidate)
                        || self.correlation.pending_responses.contains_key(*candidate)
                })
            })
            .cloned();
        let Some(stream_key) = stream_key else {
            return (None, output);
        };
        if !self.admit_correlation_stream(&stream_key, matched.response.start_time, &mut output) {
            return (None, output);
        }
        let exchanges = self
            .correlation
            .confirmed_http_exchanges
            .entry(stream_key.clone())
            .or_default();
        let response_action_id = matched.response.action_id.clone();
        if exchanges.get(&response_action_id).is_some() {
            exchanges.upsert(response_action_id, matched);
            return (Some(stream_key), output);
        }
        if exchanges.len() >= self.correlation.max_confirmed_http_exchanges_per_stream {
            if let Some(evicted) = exchanges.front() {
                output.diagnostics.push(capacity_diagnostic(
                    &evicted.response,
                    LlmPipelineDiagnosticCode::ConfirmedHttpExchangeCapacityEvicted,
                ));
            }
            exchanges.pop_front();
            tracing::warn!(
                trace_id = stream_key.trace_id.get(),
                process_id = stream_key.process.get(),
                stream_key = %stream_key.stream_key,
                stream_id = ?stream_key.http_stream_id,
                "dropped oldest unconsumed confirmed HTTP exchange at configured capacity"
            );
        }
        exchanges.upsert(response_action_id, matched);
        (Some(stream_key), output)
    }

    pub(in crate::llm_pipeline) fn reconcile_ready_http_exchange(
        &mut self,
        matched: &HttpResponseMatch,
    ) -> Option<LiveLlmOutput> {
        let ready = LlmStreamKey::from_http_response_candidates(&matched.response)
            .iter()
            .any(|candidate| self.http_exchange_ready(candidate, matched));
        if !ready {
            return None;
        }
        let (stream_key, mut output) = self.remember_confirmed_http_exchange(matched.clone());
        let stream_key = stream_key?;
        output.extend(self.reconcile_confirmed_http_exchanges(&stream_key));
        Some(output)
    }

    fn http_exchange_ready(&self, stream_key: &LlmStreamKey, matched: &HttpResponseMatch) -> bool {
        self.correlation
            .open_requests
            .get(stream_key)
            .and_then(IndexedQueue::front)
            .zip(
                self.correlation
                    .pending_responses
                    .get(stream_key)
                    .and_then(IndexedQueue::front),
            )
            .is_some_and(|(request, response)| {
                request.matches_http_request(&matched.request)
                    && response.matches_http_response(&matched.response)
            })
    }

    pub(in crate::llm_pipeline) fn reconcile_confirmed_http_exchanges(
        &mut self,
        stream_key: &LlmStreamKey,
    ) -> LiveLlmOutput {
        let ready = self
            .correlation
            .confirmed_http_exchanges
            .get(stream_key)
            .and_then(IndexedQueue::front)
            .zip(
                self.correlation
                    .open_requests
                    .get(stream_key)
                    .and_then(IndexedQueue::front),
            )
            .zip(
                self.correlation
                    .pending_responses
                    .get(stream_key)
                    .and_then(IndexedQueue::front),
            )
            .is_some_and(|((exchange, request), response)| {
                request.matches_http_request(&exchange.request)
                    && response.matches_http_response(&exchange.response)
            });
        if !ready {
            return LiveLlmOutput::default();
        }
        let Some(exchange) = self
            .correlation
            .confirmed_http_exchanges
            .get_mut(stream_key)
            .and_then(IndexedQueue::pop_front)
        else {
            return LiveLlmOutput::default();
        };
        let Some(request) = self
            .correlation
            .open_requests
            .get_mut(stream_key)
            .and_then(IndexedQueue::pop_front)
            .map(|request| request.action)
        else {
            return LiveLlmOutput::default();
        };
        let Some(mut response) = self
            .correlation
            .pending_responses
            .get_mut(stream_key)
            .and_then(IndexedQueue::pop_front)
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        let response_binding_key = (response.action.trace_id, response.action.action_id.clone());
        if let Some(owner) = self
            .correlation
            .active_response_requests
            .get(&response_binding_key)
        {
            if owner.request.action_id != request.action_id {
                tracing::warn!(
                    trace_id = response.action.trace_id.get(),
                    response_action_id = %response.action.action_id,
                    owner_request_action_id = %owner.request.action_id,
                    rejected_request_action_id = %request.action_id,
                    "rejected a second request for an active LLM response"
                );
                self.restore_open_request(request);
                return LiveLlmOutput::default();
            }
        }
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        if self
            .correlation
            .pending_responses
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.pending_responses.remove(stream_key);
        }
        if self
            .correlation
            .confirmed_http_exchanges
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.confirmed_http_exchanges.remove(stream_key);
        }
        let http_request_action_id = exchange.request.action_id;
        let http_request_evidence = exchange.request.evidence;
        let http_response = exchange.response;
        let http_response_action_id = http_response.action_id.clone();
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
        output.http_response_links.push(LlmHttpResponseLink {
            llm_response: response.action.clone(),
            http_response,
        });
        let assignments = self.projector.register_provider_response(
            &request,
            response.provider_response_id.as_deref(),
            response
                .action
                .end_time
                .unwrap_or(response.action.start_time),
        );
        self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
        if !response.compacted {
            self.push_recorded_action(response.action.clone(), &mut output);
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
        output.extend(self.update_active_response_request(
            &response.action,
            ActiveLlmResponseBinding {
                request,
                response: response.action.clone(),
                http_request_action_id,
                http_response_action_id,
            },
            response.closed,
        ));
        self.push_recorded_action(call, &mut output);
        output
    }

    pub(in crate::llm_pipeline) fn reconcile_exact_websocket_exchange(
        &mut self,
        stream_key: &LlmStreamKey,
    ) -> LiveLlmOutput {
        if !websocket::WebSocketLlmAdapter::is_exchange_stream_key(&stream_key.stream_key) {
            return LiveLlmOutput::default();
        }
        let Some(request) = self
            .correlation
            .open_requests
            .get_mut(stream_key)
            .and_then(IndexedQueue::pop_front)
            .map(|request| request.action)
        else {
            return LiveLlmOutput::default();
        };
        let Some(response) = self
            .correlation
            .pending_responses
            .get_mut(stream_key)
            .and_then(IndexedQueue::pop_front)
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        if self
            .correlation
            .pending_responses
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.pending_responses.remove(stream_key);
        }

        let mut output = LiveLlmOutput::default();
        let assignments = self.projector.register_provider_response(
            &request,
            response.provider_response_id.as_deref(),
            response
                .action
                .end_time
                .unwrap_or(response.action.start_time),
        );
        self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
        if !response.compacted {
            self.push_recorded_action(response.action.clone(), &mut output);
        }
        let call = call::llm_call_from_request_response(&request, Some(&response.action));
        self.push_recorded_action(call, &mut output);
        output
    }

    /// Completes one closed exchange after the current payload transaction had
    /// a chance to provide its exact application HTTP exchange.
    fn reconcile_closed_unconfirmed_exchange(
        &mut self,
        stream_key: &LlmStreamKey,
        response_action_id: &str,
    ) -> (LiveLlmOutput, Option<ClosedLlmExchangeBinding>) {
        if websocket::WebSocketLlmAdapter::is_exchange_stream_key(&stream_key.stream_key) {
            return (LiveLlmOutput::default(), None);
        }
        let ready = self
            .correlation
            .pending_responses
            .get(stream_key)
            .and_then(IndexedQueue::back)
            .filter(|response| response.action.action_id == response_action_id)
            .filter(|response| response.closed)
            .and_then(|response| {
                let response_order = LlmActionOrder::from_action(&response.action)?;
                let request = self.correlation.open_requests.get(stream_key)?.back()?;
                let request_order = LlmActionOrder::from_action(&request.action)?;
                (request_order <= response_order).then_some(())
            })
            .is_some();
        if !ready {
            return (LiveLlmOutput::default(), None);
        }
        let Some(response) = self
            .correlation
            .pending_responses
            .get_mut(stream_key)
            .and_then(|responses| responses.remove(response_action_id))
        else {
            return (LiveLlmOutput::default(), None);
        };
        let Some(request) = self
            .correlation
            .open_requests
            .get_mut(stream_key)
            .and_then(IndexedQueue::pop_back)
        else {
            self.correlation
                .pending_responses
                .entry(stream_key.clone())
                .or_default()
                .upsert(response.action.action_id.clone(), response);
            return (LiveLlmOutput::default(), None);
        };
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        if self
            .correlation
            .pending_responses
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.pending_responses.remove(stream_key);
        }

        let mut output = LiveLlmOutput::default();
        let assignments = self.projector.register_provider_response(
            &request.action,
            response.provider_response_id.as_deref(),
            response
                .action
                .end_time
                .unwrap_or(response.action.start_time),
        );
        self.apply_resolved_trajectory_assignments(
            request.action.trace_id,
            assignments,
            &mut output,
        );
        if !response.compacted {
            self.push_recorded_action(response.action.clone(), &mut output);
        }
        let mut llm_call =
            call::llm_call_from_request_response(&request.action, Some(&response.action));
        if response.action.status == SemanticActionStatus::Error {
            llm_call.status = SemanticActionStatus::Error;
            llm_call.completeness = SemanticActionCompleteness::Partial;
        }
        self.push_recorded_action(llm_call, &mut output);
        let binding = ClosedLlmExchangeBinding::new(request, response);
        (output, Some(binding))
    }

    pub(in crate::llm_pipeline) fn reconcile_closed_unconfirmed_identity_exchanges(
        &mut self,
        identity: &model_core::payload::PayloadStreamIdentity,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let markers = self.correlation.take_closed_pending_responses(identity);
        let mut closed_bindings = Vec::new();
        let mut blocked_markers = Vec::new();
        for marker in markers.into_iter().rev() {
            if websocket::WebSocketLlmAdapter::is_exchange_stream_key(&marker.stream_key.stream_key)
            {
                self.correlation
                    .compact_pending_response(&marker.stream_key, &marker.response_action_id);
                continue;
            }
            let (changed, binding) = self.reconcile_closed_unconfirmed_exchange(
                &marker.stream_key,
                &marker.response_action_id,
            );
            output.extend(changed);
            if let Some(binding) = binding {
                closed_bindings.push((marker.stream_key, binding));
            } else if self
                .correlation
                .compact_pending_response(&marker.stream_key, &marker.response_action_id)
            {
                blocked_markers.push(marker);
            }
        }
        for (stream_key, binding) in closed_bindings.into_iter().rev() {
            self.remember_closed_llm_exchange(&stream_key, binding, &mut output);
        }
        for marker in blocked_markers.into_iter().rev() {
            self.correlation
                .mark_closed_pending_response(&marker.stream_key, &marker.response_action_id);
        }
        output
    }

    /// Pair the LLM requests and responses that remain after exact HTTP
    /// reconciliation. Application HTTP actions can arrive late or be absent
    /// even when both LLM sides were reconstructed, so the fallback applies to
    /// every non-WebSocket stream and only consumes still-unpaired actions.
    pub(in crate::llm_pipeline) fn reconcile_unconfirmed_stream_exchanges(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let mut keys = BTreeSet::new();
        keys.extend(
            self.correlation
                .open_requests
                .keys()
                .filter(|key| key.trace_id == trace_id)
                .cloned(),
        );
        keys.extend(
            self.correlation
                .pending_responses
                .keys()
                .filter(|key| key.trace_id == trace_id)
                .cloned(),
        );
        for key in keys {
            if websocket::WebSocketLlmAdapter::is_exchange_stream_key(&key.stream_key) {
                continue;
            }
            output.extend(self.reconcile_ordered_stream(&key, finished_at));
        }
        output
    }

    pub(in crate::llm_pipeline) fn reconcile_unconfirmed_identity_exchanges(
        &mut self,
        identity: &model_core::payload::PayloadStreamIdentity,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        for key in self.correlation.unconfirmed_streams_for_identity(identity) {
            if websocket::WebSocketLlmAdapter::is_exchange_stream_key(&key.stream_key) {
                continue;
            }
            output.extend(self.reconcile_ordered_stream(&key, finished_at));
        }
        output
    }

    fn reconcile_ordered_stream(
        &mut self,
        stream_key: &LlmStreamKey,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let Some(requests) = self.correlation.open_requests.remove(stream_key) else {
            return output;
        };
        let Some(responses) = self.correlation.pending_responses.remove(stream_key) else {
            self.correlation
                .open_requests
                .insert(stream_key.clone(), requests);
            return output;
        };
        let mut requests = requests;
        let mut eligible_requests = Vec::new();
        let mut remaining_responses = IndexedQueue::new();
        for response in responses.into_values() {
            let Some(response_order) = LlmActionOrder::from_action(&response.action) else {
                remaining_responses.upsert(response.action.action_id.clone(), response);
                continue;
            };
            // Confirmed HTTP exchanges have already consumed their exact
            // requests. For the remaining HTTP/1/TLS evidence, bind a
            // response to the most recent preceding request. An older probe
            // may legitimately have no response; FIFO pairing would shift
            // every later response onto the wrong call.
            while requests.front().is_some_and(|request| {
                LlmActionOrder::from_action(&request.action)
                    .is_some_and(|request_order| request_order <= response_order)
            }) {
                if let Some(request) = requests.pop_front() {
                    eligible_requests.push(request);
                }
            }
            let Some(request) = eligible_requests.pop() else {
                remaining_responses.upsert(response.action.action_id.clone(), response);
                continue;
            };
            let request = request.action;

            let assignments = self.projector.register_provider_response(
                &request,
                response.provider_response_id.as_deref(),
                response
                    .action
                    .end_time
                    .unwrap_or(response.action.start_time),
            );
            self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
            if !response.compacted {
                self.push_recorded_action(response.action.clone(), &mut output);
            }
            let mut call = call::llm_call_from_request_response(&request, Some(&response.action));
            if response.action.status == SemanticActionStatus::Error {
                call.status = SemanticActionStatus::Error;
                call.completeness = SemanticActionCompleteness::Partial;
                call.end_time = response.action.end_time.or(Some(finished_at));
                if let Some(value) = response
                    .action
                    .attributes
                    .get(attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE)
                {
                    call.attributes.insert(
                        attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                        value.clone(),
                    );
                }
            }
            self.push_recorded_action(call, &mut output);
        }
        let mut remaining_requests = IndexedQueue::new();
        for request in eligible_requests.into_iter().chain(requests.into_values()) {
            remaining_requests.upsert(request.action.action_id.clone(), request);
        }
        if !remaining_requests.is_empty() {
            self.correlation
                .open_requests
                .insert(stream_key.clone(), remaining_requests);
        }
        if !remaining_responses.is_empty() {
            self.correlation
                .pending_responses
                .insert(stream_key.clone(), remaining_responses);
        }
        output
    }

    pub(in crate::llm_pipeline) fn take_incomplete_http1_exchange(
        &mut self,
        stream_key: &LlmStreamKey,
        operation_id: u64,
        unmatched_request: Option<MatchedHttpRequest>,
    ) -> Option<(SemanticAction, MatchedHttpRequest, Option<SemanticAction>)> {
        let exchange_selection = self
            .correlation
            .confirmed_http_exchanges
            .get(stream_key)
            .and_then(|exchanges| {
                exchanges
                    .iter()
                    .filter_map(|exchange| {
                        let response_sequence = exchange
                            .response
                            .attributes
                            .get("payload_sequence")
                            .and_then(|value| value.parse::<u64>().ok())?;
                        (response_sequence == operation_id).then_some((
                            exchange.response.action_id.clone(),
                            exchange.request.clone(),
                        ))
                    })
                    .last()
            });
        if let Some((exchange_action_id, matched_request)) = exchange_selection {
            let request_action_id = self
                .correlation
                .open_requests
                .get(stream_key)?
                .iter()
                .find(|request| request.matches_http_request(&matched_request))?
                .action
                .action_id
                .clone();
            let exchange = self
                .correlation
                .confirmed_http_exchanges
                .get_mut(stream_key)?
                .remove(&exchange_action_id)?;
            if self
                .correlation
                .confirmed_http_exchanges
                .get(stream_key)
                .is_some_and(IndexedQueue::is_empty)
            {
                self.correlation.confirmed_http_exchanges.remove(stream_key);
            }
            let request = self
                .correlation
                .open_requests
                .get_mut(stream_key)?
                .remove(&request_action_id)?
                .action;
            if self
                .correlation
                .open_requests
                .get(stream_key)
                .is_some_and(IndexedQueue::is_empty)
            {
                self.correlation.open_requests.remove(stream_key);
            }
            return Some((request, exchange.request, Some(exchange.response)));
        }
        let matched_request = unmatched_request?;
        let request_action_id = self
            .correlation
            .open_requests
            .get(stream_key)?
            .iter()
            .find(|request| request.matches_http_request(&matched_request))?
            .action
            .action_id
            .clone();
        let request = self
            .correlation
            .open_requests
            .get_mut(stream_key)?
            .remove(&request_action_id)?
            .action;
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        Some((request, matched_request, None))
    }
}

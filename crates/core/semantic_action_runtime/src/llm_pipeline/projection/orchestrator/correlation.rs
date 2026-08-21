//! HTTP and LLM request/response correlation orchestration.

use std::collections::{BTreeSet, VecDeque};
use std::time::SystemTime;

use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

use crate::live::{HttpResponseMatch, MatchedHttpRequest};
use crate::llm_pipeline::projection::correlation::{
    self as call, ActiveLlmResponseBinding, LlmActionOrder, LlmStreamKey,
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
        let candidates = LlmStreamKey::from_http_response_candidates(&matched.response);
        let stream_key = candidates
            .iter()
            .find(|candidate| {
                self.correlation.open_requests.contains_key(*candidate)
                    && self.correlation.pending_responses.contains_key(*candidate)
            })
            .or_else(|| {
                candidates.iter().find(|candidate| {
                    self.correlation.open_requests.contains_key(*candidate)
                        || self.correlation.pending_responses.contains_key(*candidate)
                })
            })
            .or_else(|| candidates.first())
            .cloned();
        let Some(stream_key) = stream_key else {
            return (None, output);
        };
        if !self.admit_correlation_stream(&stream_key, matched.response.start_time, &mut output) {
            return (None, output);
        }
        // A CONNECT exchange only marks a TLS tunnel start; it does not make
        // the stream "HTTP-visible" for confirmed-exchange binding, so those
        // streams stay eligible for the trace-close pairing fallback.
        if matched.request.method.as_deref() != Some("CONNECT") {
            self.correlation
                .http_exchange_streams
                .insert(stream_key.clone());
        }
        let exchanges = self
            .correlation
            .confirmed_http_exchanges
            .entry(stream_key.clone())
            .or_default();
        if let Some(existing) = exchanges
            .iter_mut()
            .find(|exchange| exchange.response.action_id == matched.response.action_id)
        {
            *existing = matched;
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
        exchanges.push_back(matched);
        (Some(stream_key), output)
    }

    pub(in crate::llm_pipeline) fn reconcile_confirmed_http_exchanges(
        &mut self,
        stream_key: &LlmStreamKey,
    ) -> LiveLlmOutput {
        let selection = self
            .correlation
            .confirmed_http_exchanges
            .get(stream_key)
            .and_then(|exchanges| {
                exchanges
                    .iter()
                    .enumerate()
                    .find_map(|(exchange_index, exchange)| {
                        let request_index = self
                            .correlation
                            .open_requests
                            .get(stream_key)?
                            .iter()
                            .position(|request| request.matches_http_request(&exchange.request))?;
                        let response_index = self
                            .correlation
                            .pending_responses
                            .get(stream_key)?
                            .iter()
                            .position(|response| {
                                response.matches_http_response(&exchange.response)
                            })?;
                        Some((exchange_index, request_index, response_index))
                    })
            });
        let Some((exchange_index, request_index, response_index)) = selection else {
            return LiveLlmOutput::default();
        };
        let Some(exchange) = self
            .correlation
            .confirmed_http_exchanges
            .get_mut(stream_key)
            .and_then(|exchanges| exchanges.remove(exchange_index))
        else {
            return LiveLlmOutput::default();
        };
        let Some(request) = self
            .correlation
            .open_requests
            .get_mut(stream_key)
            .and_then(|requests| requests.remove(request_index))
            .map(|request| request.action)
        else {
            return LiveLlmOutput::default();
        };
        let Some(mut response) = self
            .correlation
            .pending_responses
            .get_mut(stream_key)
            .and_then(|responses| responses.remove(response_index))
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        if self
            .correlation
            .pending_responses
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.correlation.pending_responses.remove(stream_key);
        }
        if self
            .correlation
            .confirmed_http_exchanges
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
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
        self.push_recorded_action(response.action.clone(), &mut output);
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
                http_request_action_id,
                http_response_action_id,
            },
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
            .and_then(VecDeque::pop_front)
            .map(|request| request.action)
        else {
            return LiveLlmOutput::default();
        };
        let Some(response) = self
            .correlation
            .pending_responses
            .get_mut(stream_key)
            .and_then(VecDeque::pop_front)
        else {
            self.restore_open_request(request);
            return LiveLlmOutput::default();
        };
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        if self
            .correlation
            .pending_responses
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
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
        self.push_recorded_action(response.action.clone(), &mut output);
        let call = call::llm_call_from_request_response(&request, Some(&response.action));
        self.push_recorded_action(call, &mut output);
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
        let mut requests = requests.into_iter().collect::<Vec<_>>();
        requests.sort_by_key(|request| {
            (
                LlmActionOrder::from_action(&request.action).is_none(),
                LlmActionOrder::from_action(&request.action),
                request.action.action_id.clone(),
            )
        });
        let mut responses = responses.into_iter().collect::<Vec<_>>();
        responses.sort_by_key(|response| {
            (
                LlmActionOrder::from_action(&response.action).is_none(),
                LlmActionOrder::from_action(&response.action),
                response.action.action_id.clone(),
            )
        });
        let mut requests = VecDeque::from(requests);
        let mut responses = VecDeque::from(responses);
        let mut remaining_responses = VecDeque::new();
        while let (Some(request), Some(response)) = (requests.front(), responses.front()) {
            let Some(request_order) = LlmActionOrder::from_action(&request.action) else {
                break;
            };
            let Some(response_order) = LlmActionOrder::from_action(&response.action) else {
                if let Some(response) = responses.pop_front() {
                    remaining_responses.push_back(response);
                }
                continue;
            };
            if request_order > response_order {
                if let Some(response) = responses.pop_front() {
                    remaining_responses.push_back(response);
                }
                continue;
            }
            let request = requests
                .pop_front()
                .expect("front request was present")
                .action;
            let response = responses.pop_front().expect("front response was present");

            let assignments = self.projector.register_provider_response(
                &request,
                response.provider_response_id.as_deref(),
                response
                    .action
                    .end_time
                    .unwrap_or(response.action.start_time),
            );
            self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
            self.push_recorded_action(response.action.clone(), &mut output);
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
        remaining_responses.extend(responses);
        if !requests.is_empty() {
            self.correlation
                .open_requests
                .insert(stream_key.clone(), requests);
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
                    .enumerate()
                    .rev()
                    .find_map(|(index, exchange)| {
                        let response_sequence = exchange
                            .response
                            .attributes
                            .get("payload_sequence")
                            .and_then(|value| value.parse::<u64>().ok())?;
                        (response_sequence == operation_id)
                            .then_some((index, exchange.request.clone()))
                    })
            });
        if let Some((exchange_index, matched_request)) = exchange_selection {
            let request_index =
                self.correlation
                    .open_requests
                    .get(stream_key)
                    .and_then(|requests| {
                        requests
                            .iter()
                            .position(|request| request.matches_http_request(&matched_request))
                    });
            let exchange = self
                .correlation
                .confirmed_http_exchanges
                .get_mut(stream_key)?
                .remove(exchange_index)?;
            if self
                .correlation
                .confirmed_http_exchanges
                .get(stream_key)
                .is_some_and(VecDeque::is_empty)
            {
                self.correlation.confirmed_http_exchanges.remove(stream_key);
            }
            let Some(request_index) = request_index else {
                return None;
            };
            let request = self
                .correlation
                .open_requests
                .get_mut(stream_key)?
                .remove(request_index)?
                .action;
            if self
                .correlation
                .open_requests
                .get(stream_key)
                .is_some_and(VecDeque::is_empty)
            {
                self.correlation.open_requests.remove(stream_key);
            }
            return Some((request, exchange.request, Some(exchange.response)));
        }
        let matched_request = unmatched_request?;
        let request_index = self
            .correlation
            .open_requests
            .get(stream_key)?
            .iter()
            .position(|request| request.matches_http_request(&matched_request))?;
        let request = self
            .correlation
            .open_requests
            .get_mut(stream_key)?
            .remove(request_index)?
            .action;
        if self
            .correlation
            .open_requests
            .get(stream_key)
            .is_some_and(VecDeque::is_empty)
        {
            self.correlation.open_requests.remove(stream_key);
        }
        Some((request, matched_request, None))
    }

    pub(in crate::llm_pipeline) fn take_open_request_for_http_response(
        &mut self,
        http_response: &SemanticAction,
        matched_request: &MatchedHttpRequest,
    ) -> Option<SemanticAction> {
        for stream_key in LlmStreamKey::from_http_response_candidates(http_response) {
            let Some(requests) = self.correlation.open_requests.get_mut(&stream_key) else {
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
                self.correlation.open_requests.remove(&stream_key);
            }
            return Some(request);
        }
        None
    }

    pub(in crate::llm_pipeline) fn take_open_request_for_unmatched_http_failure(
        &mut self,
        http_response: &SemanticAction,
    ) -> Option<SemanticAction> {
        for stream_key in LlmStreamKey::from_http_response_candidates(http_response) {
            let Some(requests) = self.correlation.open_requests.get_mut(&stream_key) else {
                continue;
            };
            if !requests
                .front()
                .is_some_and(|request| request.action.start_time <= http_response.start_time)
            {
                continue;
            }
            let request = requests.pop_front()?.action;
            if requests.is_empty() {
                self.correlation.open_requests.remove(&stream_key);
            }
            self.correlation.http_exchange_streams.insert(stream_key);
            return Some(request);
        }
        None
    }
}

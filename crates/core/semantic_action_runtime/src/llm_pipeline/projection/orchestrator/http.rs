//! HTTP exchange and damaged-message projection coordination.

use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionStatus, attr_keys as attrs};

use crate::live::{HttpResponseMatch, MatchedHttpRequest};
use crate::llm_pipeline::projection::correlation::{
    self as call, DamagedHttpResponseBinding, IncompleteHttp1Response, IndexedQueue,
    LateHttpFailureBinding, LlmStreamKey,
};
use crate::llm_pipeline::projection::links::{LlmHttpRequestLink, LlmHttpResponseLink};
use crate::llm_pipeline::projection::projector::{self as http, capacity_diagnostic};

use super::super::ProjectionBatch as LiveLlmOutput;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn observe_http_exchange(
        &mut self,
        matched: &HttpResponseMatch,
    ) -> LiveLlmOutput {
        if !matched.eligible_for_llm_correlation() {
            return LiveLlmOutput::default();
        }
        if http::terminal_failure_response(&matched.response)
            && matched.reconciled_late
            && let Some(output) = self.reconcile_late_http_failure(matched)
        {
            return output;
        }
        if let Some(output) = self.reconcile_late_closed_http_exchange(matched) {
            return output;
        }
        if http::terminal_failure_response(&matched.response) {
            if let Some(output) = self.reconcile_ready_http_exchange(matched) {
                return output;
            }
        } else {
            let (stream_key, mut output) = self.remember_confirmed_http_exchange(matched.clone());
            let Some(stream_key) = stream_key else {
                return output;
            };
            output.extend(self.reconcile_confirmed_http_exchanges(&stream_key));
            return output;
        }
        let action = &matched.response;
        let matched_request = &matched.request;
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
        self.projector.reject_trajectory_parent(&request);
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
        output.http_response_links.push(LlmHttpResponseLink {
            llm_response: failed_response.clone(),
            http_response: action.clone(),
        });
        self.push_recorded_action(failed_response, &mut output);
        self.push_recorded_action(failed_call, &mut output);
        output
    }

    pub(in crate::llm_pipeline) fn observe_unmatched_http_response(
        &mut self,
        action: &SemanticAction,
    ) -> LiveLlmOutput {
        self.project_unmatched_terminal_http_response(action)
            .unwrap_or_default()
    }

    fn project_unmatched_terminal_http_response(
        &mut self,
        action: &SemanticAction,
    ) -> Option<LiveLlmOutput> {
        if !http::terminal_failure_response(action) {
            return None;
        }
        let Some(request) = self.take_open_request_for_unmatched_http_failure(action) else {
            return None;
        };
        let call = call::llm_call_from_request_response(&request, None);
        let Some(failed_response) = http::failed_response_for_open_request(action, &request, &call)
        else {
            self.restore_open_request(request);
            return None;
        };
        self.projector.reject_trajectory_parent(&request);
        let mut failed_call =
            call::llm_call_from_request_response(&request, Some(&failed_response));
        failed_call.attributes.insert(
            attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
            action.action_id.clone(),
        );
        let mut output = LiveLlmOutput::default();
        let binding_key = (action.trace_id, action.action_id.clone());
        let stream_key = LlmStreamKey::from_http_response_candidates(action)
            .into_iter()
            .next();
        if let Some(stream_key) = stream_key {
            self.remember_late_http_failure_binding(
                binding_key,
                LateHttpFailureBinding::new(stream_key, &request),
                &mut output,
            );
        }
        output.http_response_links.push(LlmHttpResponseLink {
            llm_response: failed_response.clone(),
            http_response: action.clone(),
        });
        self.push_recorded_action(failed_response, &mut output);
        self.push_recorded_action(failed_call, &mut output);
        Some(output)
    }

    fn reconcile_late_http_failure(
        &mut self,
        matched: &HttpResponseMatch,
    ) -> Option<LiveLlmOutput> {
        let key = (
            matched.response.trace_id,
            matched.response.action_id.clone(),
        );
        let binding = self.take_late_http_failure_binding(&key)?;
        let call = call::llm_call_from_request_response(&binding.request, None);
        let Some(mut failed_response) =
            http::failed_response_for_open_request(&matched.response, &binding.request, &call)
        else {
            let mut output = LiveLlmOutput::default();
            output
                .diagnostics
                .push(late_http_failure_binding_gap(&matched.response));
            return Some(output);
        };
        failed_response.attributes.insert(
            attrs::http_response::REQUEST_ACTION_ID.to_string(),
            matched.request.action_id.clone(),
        );
        let mut output = LiveLlmOutput::default();
        output.http_request_links.push(LlmHttpRequestLink {
            llm_request: binding.request,
            http_request: matched.request.clone(),
        });
        self.push_recorded_action(failed_response, &mut output);
        Some(output)
    }

    pub(in crate::llm_pipeline) fn observe_damaged_http_response(
        &mut self,
        response: &SemanticAction,
    ) -> LiveLlmOutput {
        for stream_key in LlmStreamKey::from_http_response_candidates(response) {
            let matched = self
                .correlation
                .pending_responses
                .get_mut(&stream_key)
                .and_then(|responses| {
                    let action_id = responses
                        .iter()
                        .find(|candidate| candidate.matches_http_response(response))?
                        .action
                        .action_id
                        .clone();
                    let matched = responses.get_mut(&action_id)?;
                    let was_in_progress = matched.action.status == SemanticActionStatus::InProgress;
                    http::mark_response_for_http_failure(&mut matched.action, response);
                    Some((matched.action.clone(), was_in_progress))
                });
            if let Some((matched, was_in_progress)) = matched {
                let mut output = LiveLlmOutput::default();
                if was_in_progress {
                    self.remember_damaged_response_binding(
                        (matched.trace_id, matched.action_id.clone()),
                        DamagedHttpResponseBinding {
                            stream_key: stream_key.clone(),
                            http_response: response.clone(),
                        },
                        &mut output,
                    );
                }
                output.http_response_links.push(LlmHttpResponseLink {
                    llm_response: matched.clone(),
                    http_response: response.clone(),
                });
                self.push_recorded_action(matched, &mut output);
                return output;
            }
        }
        // The generic HTTP parser may retain a damaged outbound request tombstone
        // even when the LLM request projector reconstructed that request in full.
        // Only synthesize a terminal failure after ruling out an exact decoded
        // response on the same stream.
        if let Some(output) = self.project_unmatched_terminal_http_response(response) {
            return output;
        }
        let Some(stream_key) = LlmStreamKey::from_http_response_candidates(response)
            .into_iter()
            .next()
        else {
            return LiveLlmOutput::default();
        };
        let mut output = LiveLlmOutput::default();
        if !self.admit_correlation_stream(&stream_key, response.start_time, &mut output) {
            return output;
        }
        let responses = self
            .correlation
            .damaged_http_responses
            .entry(stream_key)
            .or_default();
        if responses
            .iter()
            .any(|candidate| candidate.action_id == response.action_id)
        {
            return LiveLlmOutput::default();
        }
        if responses.len() >= self.correlation.max_confirmed_http_exchanges_per_stream {
            if let Some(evicted) = responses.front() {
                output.diagnostics.push(capacity_diagnostic(
                    evicted,
                    LlmPipelineDiagnosticCode::DamagedHttpResponseCapacityEvicted,
                ));
            }
            responses.pop_front();
        }
        responses.push_back(response.clone());
        output
    }

    pub(in crate::llm_pipeline) fn prepare_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        request: Option<MatchedHttpRequest>,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let stream_key = LlmStreamKey {
            trace_id: segment.trace_id,
            process: segment.process,
            stream_key: segment.stream_key.to_string(),
            http_stream_id: None,
        };
        if !self.admit_correlation_stream(&stream_key, segment.observed_at, &mut output) {
            return output;
        }
        self.correlation
            .incomplete_http1_responses
            .insert(stream_key, IncompleteHttp1Response { sequence, request });
        output
    }

    pub(in crate::llm_pipeline) fn localize_incomplete_http1_request(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
    ) -> bool {
        let stream_key = LlmStreamKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            http_stream_id: None,
        };
        let selection = self
            .correlation
            .confirmed_http_exchanges
            .get(&stream_key)
            .and_then(|exchanges| {
                exchanges
                    .iter()
                    .filter(|exchange| exchange.request.sequence == sequence)
                    .map(|exchange| exchange.response.action_id.clone())
                    .last()
            });
        let Some(exchange_action_id) = selection else {
            return false;
        };
        let Some(exchange) = self
            .correlation
            .confirmed_http_exchanges
            .get_mut(&stream_key)
            .and_then(|exchanges| exchanges.remove(&exchange_action_id))
        else {
            return false;
        };
        if self
            .correlation
            .confirmed_http_exchanges
            .get(&stream_key)
            .is_some_and(IndexedQueue::is_empty)
        {
            self.correlation
                .confirmed_http_exchanges
                .remove(&stream_key);
        }
        if let Some(request_action_id) =
            self.correlation
                .open_requests
                .get(&stream_key)
                .and_then(|requests| {
                    requests
                        .iter()
                        .find(|request| request.matches_http_request(&exchange.request))
                        .map(|request| request.action.action_id.clone())
                })
        {
            self.correlation
                .open_requests
                .get_mut(&stream_key)
                .and_then(|requests| requests.remove(&request_action_id));
            if self
                .correlation
                .open_requests
                .get(&stream_key)
                .is_some_and(IndexedQueue::is_empty)
            {
                self.correlation.open_requests.remove(&stream_key);
            }
        }
        let output = self.observe_damaged_http_response(&exchange.response);
        self.correlation
            .localized_http1_request_outputs
            .entry(stream_key)
            .or_default()
            .extend(output);
        true
    }

    pub(in crate::llm_pipeline) fn finish_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveLlmOutput {
        let stream_key = LlmStreamKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
            http_stream_id: None,
        };
        let Some(marker) = self
            .correlation
            .incomplete_http1_responses
            .remove(&stream_key)
        else {
            tracing::warn!(
                trace_id = segment.trace_id.get(),
                process_id = segment.process.get(),
                stream_key = %segment.stream_key,
                "incomplete HTTP/1 response marker was unavailable at finalization"
            );
            return LiveLlmOutput::default();
        };
        let Some((request, http_request, http_response)) =
            self.take_incomplete_http1_exchange(&stream_key, marker.sequence, marker.request)
        else {
            return LiveLlmOutput::default();
        };
        let mut response =
            http::damaged_response_for_open_request(http_response.as_ref(), segment, &request);
        response.attributes.insert(
            attrs::http_response::REQUEST_ACTION_ID.to_string(),
            http_request.action_id.clone(),
        );
        let mut call = call::llm_call_from_request_response(&request, Some(&response));
        if let Some(http_response) = &http_response {
            call.attributes.insert(
                attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
                http_response.action_id.clone(),
            );
        }
        let mut output = LiveLlmOutput::default();
        output.http_request_links.push(LlmHttpRequestLink {
            llm_request: request,
            http_request,
        });
        if let Some(http_response) = http_response {
            output.http_response_links.push(LlmHttpResponseLink {
                llm_response: response.clone(),
                http_response,
            });
        }
        self.push_recorded_action(response, &mut output);
        self.push_recorded_action(call, &mut output);
        output
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
            return Some(request);
        }
        None
    }
}

fn late_http_failure_binding_gap(action: &SemanticAction) -> LlmPipelineDiagnostic {
    let stream_key = action
        .attributes
        .get(attrs::payload::STREAM_KEY)
        .map(String::as_str)
        .unwrap_or("late_http_failure_binding");
    LlmPipelineDiagnostic::new(
        action.trace_id,
        &action.process,
        action.end_time.unwrap_or(action.start_time),
        LlmPipelineDiagnosticCode::LateHttpFailureBindingMissing,
        LlmPipelineDiagnosticSeverity::Warning,
        LlmPipelineDiagnosticStage::Correlation,
    )
    .with_stream_key(stream_key)
    .with_discarded_entries(1)
}

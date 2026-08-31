//! Active and damaged response binding lifecycle.

use std::collections::VecDeque;

use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

use crate::live::HttpResponseMatch;
use crate::llm_pipeline::projection::correlation::{
    self as call, ActiveLlmResponseBinding, BindingAdmission, ClosedLlmExchangeBinding,
    DamagedHttpResponseBinding, IndexedQueue, LateHttpFailureBinding, LlmStreamKey,
    PendingLlmResponse,
};
use crate::llm_pipeline::projection::projector::capacity_diagnostic;

use super::super::ProjectionBatch as LiveLlmOutput;
use super::super::links::{LlmHttpRequestLink, LlmHttpResponseLink};
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn consume_damaged_http_response(
        &mut self,
        response: &SemanticAction,
        closed: bool,
    ) -> (Option<SemanticAction>, LiveLlmOutput) {
        let mut output = LiveLlmOutput::default();
        let binding_key = (response.trace_id, response.action_id.clone());
        if let Some(binding) = self.correlation.damaged_response_bindings.get(&binding_key) {
            let http_response = binding.http_response.clone();
            if closed {
                self.remove_damaged_response_binding(&binding_key);
            }
            return (Some(http_response), output);
        }
        if self.correlation.damaged_http_responses.is_empty() {
            return (None, output);
        }
        let probe = PendingLlmResponse {
            action: response.clone(),
            provider_response_id: None,
            closed,
            compacted: false,
        };
        for stream_key in LlmStreamKey::from_llm_response(response).into_iter() {
            let matched = self
                .correlation
                .damaged_http_responses
                .get_mut(&stream_key)
                .and_then(|responses| {
                    responses
                        .iter()
                        .position(|candidate| probe.matches_http_response(candidate))
                        .and_then(|index| responses.remove(index))
                });
            let Some(http_response) = matched else {
                continue;
            };
            if !closed {
                self.remember_damaged_response_binding(
                    binding_key,
                    DamagedHttpResponseBinding {
                        stream_key: stream_key.clone(),
                        http_response: http_response.clone(),
                    },
                    &mut output,
                );
            }
            if self
                .correlation
                .damaged_http_responses
                .get(&stream_key)
                .is_some_and(VecDeque::is_empty)
            {
                self.correlation.damaged_http_responses.remove(&stream_key);
            }
            return (Some(http_response), output);
        }
        (None, output)
    }

    pub(in crate::llm_pipeline) fn request_for_response_update(
        &self,
        response: &SemanticAction,
    ) -> Option<ActiveLlmResponseBinding> {
        let key = (response.trace_id, response.action_id.clone());
        self.correlation.active_response_requests.get(&key).cloned()
    }

    pub(in crate::llm_pipeline) fn update_active_response_request(
        &mut self,
        response: &SemanticAction,
        mut binding: ActiveLlmResponseBinding,
        closed: bool,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let key = (response.trace_id, response.action_id.clone());
        if let Some(existing) = self.correlation.active_response_requests.get(&key) {
            if existing.request.action_id != binding.request.action_id {
                tracing::warn!(
                    trace_id = response.trace_id.get(),
                    response_action_id = %response.action_id,
                    owner_request_action_id = %existing.request.action_id,
                    rejected_request_action_id = %binding.request.action_id,
                    "preserved the existing owner of an active LLM response"
                );
                return output;
            }
        }
        if closed {
            self.correlation.active_response_requests.remove(&key);
            self.correlation.active_binding_owners.remove(&key);
            return output;
        }
        binding.response = response.clone();
        let Some(owner_stream_key) = binding
            .request
            .attributes
            .get(attrs::payload::STREAM_KEY)
            .cloned()
        else {
            output.diagnostics.push(capacity_diagnostic(
                response,
                LlmPipelineDiagnosticCode::ActiveResponseBindingCapacityEvicted,
            ));
            return output;
        };
        match self.correlation.active_binding_owners.admit(
            &key,
            binding.request.process,
            &owner_stream_key,
        ) {
            BindingAdmission::Inserted | BindingAdmission::Existing => {}
            BindingAdmission::Evicted(evicted_key) => {
                if let Some(evicted) = self
                    .correlation
                    .active_response_requests
                    .remove(&evicted_key)
                {
                    output.diagnostics.push(capacity_diagnostic(
                        &evicted.request,
                        LlmPipelineDiagnosticCode::ActiveResponseBindingCapacityEvicted,
                    ));
                    let mut partial_call = call::llm_call_from_request_response(
                        &evicted.request,
                        Some(&evicted.response),
                    );
                    partial_call.status = SemanticActionStatus::Error;
                    partial_call.completeness = SemanticActionCompleteness::Partial;
                    partial_call.end_time = Some(response.start_time);
                    self.push_recorded_action(partial_call, &mut output);
                }
            }
            BindingAdmission::SequenceExhausted => {
                output.diagnostics.push(capacity_diagnostic(
                    response,
                    LlmPipelineDiagnosticCode::ActiveResponseBindingCapacityEvicted,
                ));
                return output;
            }
        }
        self.correlation
            .active_response_requests
            .insert(key, binding);
        output
    }

    pub(super) fn remember_closed_llm_exchange(
        &mut self,
        stream_key: &LlmStreamKey,
        binding: ClosedLlmExchangeBinding,
        output: &mut LiveLlmOutput,
    ) {
        let response_action_id = binding.response.action.action_id.clone();
        let bindings = self
            .correlation
            .closed_llm_exchanges
            .entry(stream_key.clone())
            .or_default();
        if bindings.get(&response_action_id).is_some() {
            bindings.upsert(response_action_id, binding);
            return;
        }
        if bindings.len() >= self.correlation.max_pending_responses_per_stream
            && let Some(evicted) = bindings.pop_back()
        {
            output.diagnostics.push(capacity_diagnostic(
                &evicted.response.action,
                LlmPipelineDiagnosticCode::PendingResponseCapacityEvicted,
            ));
        }
        bindings.push_front(response_action_id, binding);
    }

    pub(in crate::llm_pipeline) fn reconcile_late_closed_http_exchange(
        &mut self,
        matched: &HttpResponseMatch,
    ) -> Option<LiveLlmOutput> {
        for stream_key in LlmStreamKey::from_http_response_candidates(&matched.response) {
            loop {
                let Some((ready, stale)) = self
                    .correlation
                    .closed_llm_exchanges
                    .get(&stream_key)
                    .and_then(IndexedQueue::back)
                    .map(|binding| {
                        (
                            binding.matches_http_exchange(matched),
                            binding.precedes_http_exchange(matched),
                        )
                    })
                else {
                    break;
                };
                if ready {
                    break;
                }
                if !stale {
                    break;
                }
                let Some(discarded) = self
                    .correlation
                    .closed_llm_exchanges
                    .get_mut(&stream_key)
                    .and_then(IndexedQueue::pop_back)
                else {
                    break;
                };
                tracing::warn!(
                    trace_id = stream_key.trace_id.get(),
                    process_id = stream_key.process.get(),
                    stream_key = %stream_key.stream_key,
                    stream_id = ?stream_key.http_stream_id,
                    llm_response_action_id = %discarded.response.action.action_id,
                    http_response_action_id = %matched.response.action_id,
                    "discarded a stale delayed-HTTP LLM binding"
                );
            }
            let ready = self
                .correlation
                .closed_llm_exchanges
                .get(&stream_key)
                .and_then(IndexedQueue::back)
                .is_some_and(|binding| binding.matches_http_exchange(matched));
            if !ready {
                if self
                    .correlation
                    .closed_llm_exchanges
                    .get(&stream_key)
                    .is_some_and(IndexedQueue::is_empty)
                {
                    self.correlation.closed_llm_exchanges.remove(&stream_key);
                }
                continue;
            }
            let binding = self
                .correlation
                .closed_llm_exchanges
                .get_mut(&stream_key)
                .and_then(IndexedQueue::pop_back)?;
            if self
                .correlation
                .closed_llm_exchanges
                .get(&stream_key)
                .is_some_and(IndexedQueue::is_empty)
            {
                self.correlation.closed_llm_exchanges.remove(&stream_key);
            }
            let request = binding.request.action;
            let response = binding.response.action;
            let mut output = LiveLlmOutput::default();
            output.http_request_links.push(LlmHttpRequestLink {
                llm_request: request.clone(),
                http_request: matched.request.clone(),
            });
            output.http_response_links.push(LlmHttpResponseLink {
                llm_response: response.clone(),
                http_response: matched.response.clone(),
            });
            let mut llm_call = call::llm_call_from_request_response(&request, Some(&response));
            llm_call.attributes.insert(
                attrs::llm_call::HTTP_RESPONSE_ACTION_ID.to_string(),
                matched.response.action_id.clone(),
            );
            if response.status == SemanticActionStatus::Error {
                llm_call.status = SemanticActionStatus::Error;
                llm_call.completeness = SemanticActionCompleteness::Partial;
            }
            self.push_recorded_action(llm_call, &mut output);
            return Some(output);
        }
        None
    }

    pub(super) fn remember_damaged_response_binding(
        &mut self,
        key: (TraceId, String),
        binding: DamagedHttpResponseBinding,
        output: &mut LiveLlmOutput,
    ) {
        match self.correlation.damaged_binding_owners.admit(
            &key,
            binding.stream_key.process,
            &binding.stream_key.stream_key,
        ) {
            BindingAdmission::Inserted | BindingAdmission::Existing => {}
            BindingAdmission::Evicted(evicted_key) => {
                if let Some(evicted) = self
                    .correlation
                    .damaged_response_bindings
                    .remove(&evicted_key)
                {
                    output.diagnostics.push(capacity_diagnostic(
                        &evicted.http_response,
                        LlmPipelineDiagnosticCode::DamagedResponseBindingCapacityEvicted,
                    ));
                }
            }
            BindingAdmission::SequenceExhausted => {
                output.diagnostics.push(capacity_diagnostic(
                    &binding.http_response,
                    LlmPipelineDiagnosticCode::DamagedResponseBindingCapacityEvicted,
                ));
                return;
            }
        }
        self.correlation
            .damaged_response_bindings
            .insert(key, binding);
    }

    fn remove_damaged_response_binding(&mut self, key: &(TraceId, String)) {
        self.correlation.damaged_binding_owners.remove(key);
        self.correlation.damaged_response_bindings.remove(key);
    }

    pub(super) fn remember_late_http_failure_binding(
        &mut self,
        key: (TraceId, String),
        binding: LateHttpFailureBinding,
        output: &mut LiveLlmOutput,
    ) {
        match self.correlation.late_failure_binding_owners.admit(
            &key,
            binding.stream_key.process,
            &binding.stream_key.stream_key,
        ) {
            BindingAdmission::Inserted | BindingAdmission::Existing => {}
            BindingAdmission::Evicted(evicted_key) => {
                if let Some(evicted) = self
                    .correlation
                    .late_http_failure_bindings
                    .remove(&evicted_key)
                {
                    output.diagnostics.push(capacity_diagnostic(
                        &evicted.request,
                        LlmPipelineDiagnosticCode::LateHttpFailureBindingCapacityEvicted,
                    ));
                }
            }
            BindingAdmission::SequenceExhausted => {
                output.diagnostics.push(capacity_diagnostic(
                    &binding.request,
                    LlmPipelineDiagnosticCode::LateHttpFailureBindingCapacityEvicted,
                ));
                return;
            }
        }
        self.correlation
            .late_http_failure_bindings
            .insert(key, binding);
    }

    pub(super) fn take_late_http_failure_binding(
        &mut self,
        key: &(TraceId, String),
    ) -> Option<LateHttpFailureBinding> {
        self.correlation.late_failure_binding_owners.remove(key);
        self.correlation.late_http_failure_bindings.remove(key)
    }
}

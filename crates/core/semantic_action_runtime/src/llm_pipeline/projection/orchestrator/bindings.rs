//! Active and damaged response binding lifecycle.

use std::collections::VecDeque;

use model_core::diagnostics::LlmPipelineDiagnosticCode;
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

use crate::llm_pipeline::projection::correlation::{
    self as call, ActiveLlmResponseBinding, BindingAdmission, DamagedHttpResponseBinding,
    LateHttpFailureBinding, LlmStreamKey, PendingLlmResponse,
};
use crate::llm_pipeline::projection::projector::capacity_diagnostic;

use super::super::ProjectionBatch as LiveLlmOutput;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn consume_damaged_http_response(
        &mut self,
        response: &SemanticAction,
    ) -> (Option<SemanticAction>, LiveLlmOutput) {
        let mut output = LiveLlmOutput::default();
        let binding_key = (response.trace_id, response.action_id.clone());
        if let Some(binding) = self.correlation.damaged_response_bindings.get(&binding_key) {
            let http_response = binding.http_response.clone();
            if response.status != SemanticActionStatus::InProgress {
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
            if response.status == SemanticActionStatus::InProgress {
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
        &mut self,
        response: &SemanticAction,
    ) -> Option<ActiveLlmResponseBinding> {
        let key = (response.trace_id, response.action_id.clone());
        self.correlation.active_binding_owners.remove(&key);
        self.correlation.active_response_requests.remove(&key)
    }

    pub(in crate::llm_pipeline) fn update_active_response_request(
        &mut self,
        response: &SemanticAction,
        binding: ActiveLlmResponseBinding,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let key = (response.trace_id, response.action_id.clone());
        if response.status == SemanticActionStatus::InProgress {
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
                        let mut partial_call =
                            call::llm_call_from_request_response(&evicted.request, None);
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
        } else {
            self.correlation.active_binding_owners.remove(&key);
            self.correlation.active_response_requests.remove(&key);
        }
        output
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
                        &evicted.failed_response,
                        LlmPipelineDiagnosticCode::LateHttpFailureBindingCapacityEvicted,
                    ));
                }
            }
            BindingAdmission::SequenceExhausted => {
                output.diagnostics.push(capacity_diagnostic(
                    &binding.failed_response,
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

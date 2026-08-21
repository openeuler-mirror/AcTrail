//! Bounded pending LLM request and response state.

use model_core::diagnostics::LlmPipelineDiagnosticCode;
use semantic_action::{SemanticAction, SemanticActionCompleteness, SemanticActionStatus};

use crate::llm_pipeline::projection::correlation::{
    self as call, LlmStreamKey, OpenLlmRequest, PendingLlmResponse,
};
use crate::llm_pipeline::projection::projector::capacity_diagnostic;

use super::super::ProjectionBatch as LiveLlmOutput;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn remember_open_request(
        &mut self,
        request: SemanticAction,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let Some(stream_key) = LlmStreamKey::from_llm_request(&request) else {
            return output;
        };
        let Some(sequence_start) = call::payload_sequence_start(&request) else {
            return output;
        };
        let Some(sequence_end) = call::payload_sequence_end(&request) else {
            return output;
        };
        let inserted_at = request.start_time;
        if !self.admit_correlation_stream(&stream_key, inserted_at, &mut output) {
            return output;
        }
        let requests = self
            .correlation
            .open_requests
            .entry(stream_key)
            .or_default();
        if let Some(existing) = requests
            .iter_mut()
            .find(|candidate| candidate.action.action_id == request.action_id)
        {
            existing.action = request;
            existing.sequence_start = sequence_start;
            existing.sequence_end = sequence_end;
            return output;
        }

        let evicted = (requests.len() >= self.correlation.max_pending_requests_per_stream)
            .then(|| requests.pop_front())
            .flatten();
        requests.push_back(OpenLlmRequest {
            action: request,
            sequence_start,
            sequence_end,
        });
        if let Some(evicted) = evicted {
            output.diagnostics.push(capacity_diagnostic(
                &evicted.action,
                LlmPipelineDiagnosticCode::PendingRequestCapacityEvicted,
            ));
            let mut partial_call = call::llm_call_from_request_response(&evicted.action, None);
            partial_call.status = SemanticActionStatus::Error;
            partial_call.completeness = SemanticActionCompleteness::Partial;
            partial_call.end_time = Some(inserted_at);
            self.push_recorded_action(partial_call, &mut output);
        }
        output
    }

    pub(in crate::llm_pipeline) fn update_open_request(&mut self, request: &SemanticAction) {
        let Some(stream_key) = LlmStreamKey::from_llm_request(request) else {
            return;
        };
        let Some(requests) = self.correlation.open_requests.get_mut(&stream_key) else {
            return;
        };
        if let Some(existing) = requests
            .iter_mut()
            .find(|candidate| candidate.action.action_id == request.action_id)
        {
            existing.action = request.clone();
        }
    }

    pub(in crate::llm_pipeline) fn restore_open_request(&mut self, request: SemanticAction) {
        let Some(stream_key) = LlmStreamKey::from_llm_request(&request) else {
            return;
        };
        let Some(sequence_start) = call::payload_sequence_start(&request) else {
            return;
        };
        let Some(sequence_end) = call::payload_sequence_end(&request) else {
            return;
        };
        let requests = self
            .correlation
            .open_requests
            .entry(stream_key)
            .or_default();
        debug_assert!(requests.len() < self.correlation.max_pending_requests_per_stream);
        requests.push_front(OpenLlmRequest {
            action: request,
            sequence_start,
            sequence_end,
        });
    }

    pub(in crate::llm_pipeline) fn remember_pending_response(
        &mut self,
        response: SemanticAction,
        provider_response_id: Option<String>,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        let Some(stream_key) = LlmStreamKey::from_llm_response(&response) else {
            return output;
        };
        if !self.admit_correlation_stream(&stream_key, response.start_time, &mut output) {
            return output;
        }
        let responses = self
            .correlation
            .pending_responses
            .entry(stream_key)
            .or_default();
        if let Some(existing) = responses
            .iter()
            .position(|candidate| candidate.action.action_id == response.action_id)
        {
            let previous_provider_response_id = responses[existing].provider_response_id.take();
            responses[existing] = PendingLlmResponse {
                action: response,
                provider_response_id: provider_response_id.or(previous_provider_response_id),
            };
            return output;
        }
        if responses.len() >= self.correlation.max_pending_responses_per_stream
            && let Some(evicted) = responses.pop_front()
        {
            output.diagnostics.push(capacity_diagnostic(
                &evicted.action,
                LlmPipelineDiagnosticCode::PendingResponseCapacityEvicted,
            ));
        }
        responses.push_back(PendingLlmResponse {
            action: response,
            provider_response_id,
        });
        output
    }
}

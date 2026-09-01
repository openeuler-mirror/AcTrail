//! Projection/correlation lifecycle operations exposed to the facade.

use model_core::ids::TraceId;
use model_core::payload::PayloadStreamIdentity;
use semantic_action::SemanticAction;

use super::super::ProjectionBatch;
use super::super::correlation::LlmStreamKey;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn take_localized_output(
        &mut self,
        stream_key: &LlmStreamKey,
    ) -> ProjectionBatch {
        self.correlation
            .localized_http1_request_outputs
            .remove(stream_key)
            .unwrap_or_default()
    }

    pub(in crate::llm_pipeline) fn open_requests_for_identity(
        &self,
        identity: &PayloadStreamIdentity,
    ) -> Vec<SemanticAction> {
        let stream_key = identity.stream_key.to_string();
        self.correlation
            .open_requests
            .iter()
            .filter(|(key, _)| {
                key.trace_id == identity.trace_id
                    && key.process == identity.process
                    && key.stream_key == stream_key
            })
            .flat_map(|(_, requests)| requests.iter().map(|request| request.action.clone()))
            .collect()
    }

    pub(in crate::llm_pipeline) fn open_requests_for_trace(
        &self,
        trace_id: TraceId,
    ) -> Vec<SemanticAction> {
        self.correlation.open_requests_for_trace(trace_id)
    }

    pub(in crate::llm_pipeline) fn finalize_trajectory(
        &mut self,
        trace_id: TraceId,
        output: &mut ProjectionBatch,
    ) {
        let Some(classifier) = self.projector.trajectory.as_mut() else {
            return;
        };
        let assignments = classifier.finalize_trace(trace_id);
        self.apply_resolved_trajectory_assignments(trace_id, assignments, output);
    }

    pub(in crate::llm_pipeline) fn forget_identity(&mut self, identity: &PayloadStreamIdentity) {
        self.correlation.forget_stream_identity(identity);
    }

    pub(in crate::llm_pipeline) fn forget_trace_state(&mut self, trace_id: TraceId) {
        if let Some(trajectory) = self.projector.trajectory.as_mut() {
            trajectory.forget_trace(trace_id);
        }
        self.correlation
            .open_requests
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .pending_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .confirmed_http_exchanges
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .closed_llm_exchanges
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .incomplete_http1_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .damaged_http_responses
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .damaged_response_bindings
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.correlation
            .damaged_binding_owners
            .forget_trace(trace_id);
        self.correlation
            .late_http_failure_bindings
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.correlation
            .late_failure_binding_owners
            .forget_trace(trace_id);
        self.correlation
            .localized_http1_request_outputs
            .retain(|key, _| key.trace_id != trace_id);
        self.correlation
            .active_response_requests
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.correlation
            .active_binding_owners
            .forget_trace(trace_id);
        self.correlation.forget_closed_pending_trace(trace_id);
        self.correlation.stream_owners.forget_trace(trace_id);
        self.projector.forget_trace(trace_id);
    }
}

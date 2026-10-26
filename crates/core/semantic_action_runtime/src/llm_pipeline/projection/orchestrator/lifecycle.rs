//! Projection/correlation lifecycle operations exposed to the facade.

use model_core::ids::TraceId;
use semantic_action::SemanticAction;

use super::super::ProjectionBatch;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn observe_response_trajectory(
        &mut self,
        request: &SemanticAction,
        provider_response_id: Option<&str>,
        observed_at: std::time::SystemTime,
        failed: bool,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        if failed {
            self.projector.reject_trajectory_parent(request);
        } else {
            let assignments = self.projector.register_provider_response(
                request,
                provider_response_id,
                observed_at,
            );
            self.apply_resolved_trajectory_assignments(request.trace_id, assignments, &mut output);
        }
        output
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

    pub(in crate::llm_pipeline) fn forget_trace_state(&mut self, trace_id: TraceId) {
        if let Some(trajectory) = self.projector.trajectory.as_mut() {
            trajectory.forget_trace(trace_id);
        }
        self.projector.forget_trace(trace_id);
    }
}

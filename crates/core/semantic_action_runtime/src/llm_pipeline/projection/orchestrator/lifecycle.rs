//! Projection/correlation lifecycle operations exposed to the facade.

use model_core::ids::TraceId;
use semantic_action::SemanticAction;

use super::super::ProjectionBatch;
use super::super::trajectory::TrajectoryAssignment;
use super::ProjectionCoordinator;

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn observe_response_binding(
        &mut self,
        binding: super::super::ProjectedResponseBinding,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        let mut assignments = self.response_trajectory_assignments(
            &binding.request,
            binding.provider_response_id.as_deref(),
            binding.observed_at,
            binding.failed,
        );
        if let Some(compact) = self.compact.as_mut()
            && let Some(response) = binding.completion
        {
            let projection =
                compact.observe_completed_call(&binding.request, response, &assignments);
            self.push_compact_projection(projection, &mut assignments, &mut output);
            self.publish_resolved_trajectory_assignments(
                binding.request.trace_id,
                assignments,
                &mut output,
            );
        } else {
            self.apply_resolved_trajectory_assignments(
                binding.request.trace_id,
                assignments,
                &mut output,
            );
        }
        output
    }
    fn response_trajectory_assignments(
        &mut self,
        request: &SemanticAction,
        provider_response_id: Option<&str>,
        observed_at: std::time::SystemTime,
        failed: bool,
    ) -> Vec<TrajectoryAssignment> {
        if failed {
            self.projector.reject_trajectory_parent(request);
            Vec::new()
        } else {
            self.projector
                .register_provider_response(request, provider_response_id, observed_at)
        }
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
        if let Some(compact) = self.compact.as_mut() {
            compact.forget_trace(trace_id);
        }
        if let Some(trajectory) = self.projector.trajectory.as_mut() {
            trajectory.forget_trace(trace_id);
        }
        self.projector.forget_trace(trace_id);
    }
}

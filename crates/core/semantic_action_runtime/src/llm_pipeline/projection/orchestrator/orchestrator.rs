//! Projection and correlation ownership behind one facade-facing component.

use config_core::daemon::SemanticRetentionConfig;
use semantic_action::SemanticAction;

use super::super::ProjectionBatch;
use super::super::correlation::CorrelationCoordinator;
use super::super::projector::ActionProjector;

pub(in crate::llm_pipeline) struct ProjectionCoordinator {
    pub(super) correlation: CorrelationCoordinator,
    pub(super) projector: ActionProjector,
}

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn new(
        config: &SemanticRetentionConfig,
        max_confirmed_http_exchanges_per_stream: usize,
    ) -> Self {
        Self {
            correlation: CorrelationCoordinator::new(
                max_confirmed_http_exchanges_per_stream,
                config.l0_llm_call.projection_state,
            ),
            projector: ActionProjector::new(config),
        }
    }

    pub(super) fn push_recorded_action(
        &mut self,
        action: SemanticAction,
        output: &mut ProjectionBatch,
    ) {
        let record = self.projector.record_action(&action);
        output.diagnostics.extend(record.diagnostic);
        if record.changed {
            output.actions.push(action);
        }
    }
}

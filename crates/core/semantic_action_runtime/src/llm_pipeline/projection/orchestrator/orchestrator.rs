//! Projection and correlation ownership behind one facade-facing component.

use config_core::daemon::SemanticRetentionConfig;
use semantic_action::SemanticAction;

use super::super::ProjectionBatch;
use super::super::projector::ActionProjector;

pub(in crate::llm_pipeline) struct ProjectionCoordinator {
    pub(super) projector: ActionProjector,
}

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
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
            if record.existing {
                output
                    .updates
                    .push(crate::live::ActionUpdateFactory::lifecycle(&action, None));
                output.updated_actions.push(action);
            } else {
                output.actions.push(action);
            }
        }
    }
}

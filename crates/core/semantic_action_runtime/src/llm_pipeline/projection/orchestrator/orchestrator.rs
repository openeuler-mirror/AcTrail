//! Projection and correlation ownership behind one facade-facing component.

use super::super::trajectory::TrajectoryAssignment;
use config_core::daemon::SemanticRetentionConfig;
use semantic_action::SemanticAction;

use super::super::ProjectionBatch;
use super::super::compact::CompactProjectionBatch;
use super::super::compact::ContextCompactProjector;
use super::super::projector::ActionProjector;

pub(in crate::llm_pipeline) struct ProjectionCoordinator {
    pub(super) compact: Option<Box<ContextCompactProjector>>,
    pub(super) projector: ActionProjector,
}

impl ProjectionCoordinator {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
            compact: config.l0_llm_call.context_compact.enabled.then(|| {
                Box::new(ContextCompactProjector::new(
                    config.l0_llm_call.context_compact,
                    config.llm_trajectory_enabled(),
                ))
            }),
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

    pub(super) fn push_compact_projection(
        &mut self,
        projection: CompactProjectionBatch,
        assignments: &mut [TrajectoryAssignment],
        output: &mut ProjectionBatch,
    ) {
        if let Some(classifier) = self.projector.trajectory.as_mut() {
            for adoption in projection.confirmed_adoptions {
                let Some(assignment) = classifier.refine_compaction_start_reason(
                    adoption.trace_id,
                    adoption.process,
                    adoption.classifier_id,
                    &adoption.action_id,
                ) else {
                    continue;
                };
                if let Some(current) = assignments.iter_mut().find(|current| {
                    current.action_id == assignment.action_id
                        && current.trajectory_id == assignment.trajectory_id
                        && current.transition == assignment.transition
                }) {
                    current.start_reason = assignment.start_reason;
                }
            }
        }
        output.actions.extend(projection.actions);
        output.updates.extend(projection.updates);
        output.updated_actions.extend(projection.updated_actions);
        output
            .context_compact_links
            .extend(projection.implementation_links);
        output.diagnostics.extend(projection.diagnostics);
    }
}

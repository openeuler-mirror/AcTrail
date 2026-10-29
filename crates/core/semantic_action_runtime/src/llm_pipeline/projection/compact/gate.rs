use semantic_action::{LlmTrajectoryStartReason, LlmTrajectoryTransition};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use crate::llm_pipeline::projection::trajectory::TrajectoryAssignment;

pub(super) struct TrajectoryFact {
    pub(super) trajectory_id: String,
    pub(super) transition: LlmTrajectoryTransition,
    pub(super) start_reason: LlmTrajectoryStartReason,
}

pub(super) struct TrajectoryBoundaryGate;

/// All retained views of a request share its once-resolved boundary fact.
/// The classifier fixes identity and transition on assignment. A later compact
/// start-reason annotation has the same boundary eligibility as Unspecified.
#[derive(Clone)]
pub(super) struct TrajectorySlot(Arc<OnceLock<TrajectoryFact>>);

impl TrajectorySlot {
    pub(super) fn new(fact: Option<TrajectoryFact>) -> Self {
        let slot = OnceLock::new();
        if let Some(fact) = fact {
            let _ = slot.set(fact);
        }
        Self(Arc::new(slot))
    }

    pub(super) fn resolve(&self, assignment: &TrajectoryAssignment) -> bool {
        if self.0.get().is_some() {
            return false;
        }
        self.0.set(TrajectoryFact::from(assignment)).is_ok()
    }

    pub(super) fn as_ref(&self) -> Option<&TrajectoryFact> {
        self.0.get()
    }

    pub(super) fn is_none(&self) -> bool {
        self.as_ref().is_none()
    }
}

impl TrajectoryFact {
    pub(super) fn allows_adoption(&self) -> bool {
        matches!(
            self.transition,
            LlmTrajectoryTransition::Root | LlmTrajectoryTransition::ForkRoot
        ) && TrajectoryBoundaryGate::trustworthy(self)
    }
    pub(super) fn allows_inferred_candidate(&self, root_only: bool) -> bool {
        !root_only
            || matches!(
                self.transition,
                LlmTrajectoryTransition::Root
                    | LlmTrajectoryTransition::ForkRoot
                    | LlmTrajectoryTransition::DuplicateRoot
            )
    }
}

impl TrajectoryBoundaryGate {
    pub(super) fn precedes_adoption(
        &self,
        completed_at: Option<SystemTime>,
        started_at: SystemTime,
        previous: Option<&TrajectoryFact>,
        current: Option<&TrajectoryFact>,
    ) -> bool {
        completed_at.is_some_and(|end| end <= started_at)
            && match (previous, current) {
                (Some(previous), Some(current)) => self.is_candidate(Some(previous), Some(current)),
                _ => true,
            }
    }

    pub(super) fn is_candidate(
        &self,
        previous: Option<&TrajectoryFact>,
        current: Option<&TrajectoryFact>,
    ) -> bool {
        let (Some(previous), Some(current)) = (previous, current) else {
            return false;
        };
        previous.trajectory_id != current.trajectory_id
            && matches!(
                current.transition,
                LlmTrajectoryTransition::Root | LlmTrajectoryTransition::ForkRoot
            )
            && Self::trustworthy(previous)
            && Self::trustworthy(current)
    }

    fn trustworthy(fact: &TrajectoryFact) -> bool {
        matches!(
            fact.start_reason,
            LlmTrajectoryStartReason::Unspecified
                | LlmTrajectoryStartReason::ContextRewriteOrCompression
        )
    }
}

impl From<&TrajectoryAssignment> for TrajectoryFact {
    fn from(assignment: &TrajectoryAssignment) -> Self {
        Self {
            trajectory_id: assignment.trajectory_id.clone(),
            transition: assignment.transition,
            start_reason: assignment.start_reason,
        }
    }
}

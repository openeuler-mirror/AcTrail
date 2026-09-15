//! Idle detector runtime: task windows, protection, deadlines, persistence ops.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use idle_contract::IdleStoreOp;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, SemanticActionLink, attr_keys};

use self::progress::ProgressProjection;
use super::task_state::TaskIdleState;

mod attribution;
mod lifecycle;
mod persistence;
mod progress;

pub(super) const FINAL_ACTION_GRACE: Duration = Duration::from_secs(1);

/// Per-trace, per-root-task idle detector.
#[derive(Clone)]
pub struct IdleDetector {
    idle_threshold: Duration,
    tasks: BTreeMap<(TraceId, String), TaskIdleState>,
    /// Tracks processes identified as agent processes for each trace.
    agent_processes: BTreeSet<(TraceId, ProcessIdentity)>,
    progress_projection: ProgressProjection,
    /// Queued interval storage changes.
    pending_ops: Vec<IdleStoreOp>,
    /// Next idle interval ID.
    next_interval_id: u64,
}

impl IdleDetector {
    pub fn new(idle_threshold: Duration, initial_interval_id: u64) -> Self {
        Self {
            idle_threshold,
            tasks: BTreeMap::new(),
            agent_processes: BTreeSet::new(),
            progress_projection: ProgressProjection::default(),
            pending_ops: Vec::new(),
            next_interval_id: initial_interval_id,
        }
    }

    /// Attributes a semantic-action batch and applies its observable progress to idle tasks.
    pub fn observe_batch(
        &mut self,
        actions: &mut [SemanticAction],
        links: &[SemanticActionLink],
        observed_at: SystemTime,
    ) -> BTreeSet<TraceId> {
        let ambiguous_traces = self.attribute_batch_actions(actions, links);
        let mut updates = Vec::new();
        self.progress_projection
            .project_batch(actions.iter(), observed_at, |update| updates.push(update));
        for update in updates {
            // `attribute_batch_actions` resolved and recorded this once above.
            let task_id = update
                .action
                .attributes
                .get(attr_keys::agent_turn::TASK_ID)
                .cloned();
            self.on_update(update, task_id);
        }
        ambiguous_traces
    }
}

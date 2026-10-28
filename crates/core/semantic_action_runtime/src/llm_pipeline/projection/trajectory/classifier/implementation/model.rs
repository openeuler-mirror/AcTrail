//! Configuration and results of trajectory classification.

use std::time::Duration;

use config_core::daemon::LlmTrajectoryConfig;
use semantic_action::{LlmTrajectoryStartReason, LlmTrajectoryTransition};

#[derive(Clone, Debug)]
pub(in crate::llm_pipeline) struct TrajectoryClassifierConfig {
    pub(super) max_active_trajectories_per_scope: usize,
    pub(super) max_candidate_nodes_per_trajectory: usize,
    pub(super) max_prefix_nodes_per_scope: usize,
    pub(super) max_history_atoms_per_request: usize,
    pub(super) max_blocks_per_atom: usize,
    pub(super) max_structural_bytes_per_atom: usize,
    pub(super) idle_ttl: Duration,
}

impl From<LlmTrajectoryConfig> for TrajectoryClassifierConfig {
    fn from(config: LlmTrajectoryConfig) -> Self {
        Self {
            max_active_trajectories_per_scope: config.max_active_trajectories_per_scope as usize,
            max_candidate_nodes_per_trajectory: config.max_candidate_nodes_per_trajectory as usize,
            max_prefix_nodes_per_scope: config.max_prefix_nodes_per_scope as usize,
            max_history_atoms_per_request: config.max_history_atoms_per_request as usize,
            max_blocks_per_atom: config.max_blocks_per_atom as usize,
            max_structural_bytes_per_atom: config.max_structural_bytes_per_atom as usize,
            idle_ttl: config.idle_ttl,
        }
    }
}

impl TrajectoryClassifierConfig {
    pub(super) fn assignment_cache_capacity(&self) -> usize {
        let candidate_capacity = self
            .max_active_trajectories_per_scope
            .saturating_mul(self.max_candidate_nodes_per_trajectory);
        self.max_prefix_nodes_per_scope
            .max(candidate_capacity)
            .saturating_add(self.max_active_trajectories_per_scope)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct TrajectoryAssignment {
    pub(in crate::llm_pipeline) action_id: String,
    pub(in crate::llm_pipeline) trajectory_id: String,
    pub(in crate::llm_pipeline) parent_action_id: Option<String>,
    pub(in crate::llm_pipeline) forked_from_action_id: Option<String>,
    pub(in crate::llm_pipeline) position: u32,
    pub(in crate::llm_pipeline) transition: LlmTrajectoryTransition,
    pub(in crate::llm_pipeline) start_reason: LlmTrajectoryStartReason,
    pub(in crate::llm_pipeline) inference_version: u32,
}

pub(in crate::llm_pipeline) enum TrajectoryClassification {
    Assigned(TrajectoryAssignment),
    Deferred,
}

//! Bounded trajectory classification for projected LLM requests.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{Duration, SystemTime};

use config_core::daemon::LlmTrajectoryConfig;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{LlmRequestLineageWrite, LlmTrajectoryStartReason, LlmTrajectoryTransition};

use crate::llm_pipeline::projection::projector::{
    ProjectedLlmRequestHistory, ProviderContextReference,
};
use crate::llm_pipeline::projection::retention::{HistoryAtom, TrajectoryHistoryProjection};

mod prefix;
mod provider;

use prefix::PrefixTrie;

pub(super) const INFERENCE_VERSION: u32 = 2;

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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TrajectoryScope {
    trace_id: TraceId,
    process: ProcessIdentity,
    classifier_id: String,
}

impl TrajectoryScope {
    fn new(trace_id: TraceId, process: ProcessIdentity, classifier_id: String) -> Self {
        Self {
            trace_id,
            process,
            classifier_id,
        }
    }
}

pub(in crate::llm_pipeline) struct TrajectoryClassifier {
    config: TrajectoryClassifierConfig,
    scopes: HashMap<TrajectoryScope, ScopeState>,
    resolved: Vec<TrajectoryAssignment>,
}

impl TrajectoryClassifier {
    pub(in crate::llm_pipeline) fn new(config: TrajectoryClassifierConfig) -> Self {
        Self {
            config,
            scopes: HashMap::new(),
            resolved: Vec::new(),
        }
    }

    pub(in crate::llm_pipeline) fn classify(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        projected: ProjectedLlmRequestHistory,
        observed_at: SystemTime,
    ) -> TrajectoryClassification {
        let scope = TrajectoryScope::new(trace_id, process, projected.classifier_id);
        let state = self.scopes.entry(scope).or_default();
        self.resolved
            .extend(state.expire_deferred(observed_at, &self.config));
        if let Some(assignment) = state.assignments.get(&projected.action_id) {
            return TrajectoryClassification::Assigned(assignment.clone());
        }
        if state.deferred_actions.contains_key(&projected.action_id) {
            return TrajectoryClassification::Deferred;
        }
        state.expire_idle(observed_at, self.config.idle_ttl);
        match &projected.provider_context {
            ProviderContextReference::NotProvider => {}
            ProviderContextReference::Root => {
                let assignment = state.classify_provider_reference(
                    projected.action_id,
                    None,
                    observed_at,
                    &self.config,
                );
                state.remember_assignment(
                    assignment.clone(),
                    self.config.assignment_cache_capacity(),
                );
                return TrajectoryClassification::Assigned(assignment);
            }
            ProviderContextReference::PreviousResponse(previous_response_id) => {
                let valid = previous_response_id.len() <= self.config.max_structural_bytes_per_atom;
                if valid
                    && !state.provider_bindings.contains_key(previous_response_id)
                    && !state.ambiguous_provider_ids.contains(previous_response_id)
                    && state.defer_provider_request(
                        projected.action_id.clone(),
                        previous_response_id,
                        observed_at,
                        self.config.assignment_cache_capacity(),
                    )
                {
                    return TrajectoryClassification::Deferred;
                }
                let assignment = state.classify_provider_reference(
                    projected.action_id,
                    valid.then_some(previous_response_id.as_str()),
                    observed_at,
                    &self.config,
                );
                state.remember_assignment(
                    assignment.clone(),
                    self.config.assignment_cache_capacity(),
                );
                return TrajectoryClassification::Assigned(assignment);
            }
        }
        let assignment = match projected.history {
            TrajectoryHistoryProjection::UnsupportedMultimodal => singleton_assignment(
                projected.action_id,
                LlmTrajectoryStartReason::UnsupportedMultimodal,
            ),
            TrajectoryHistoryProjection::Supported(history)
                if history.len() > self.config.max_history_atoms_per_request =>
            {
                singleton_assignment(projected.action_id, LlmTrajectoryStartReason::HistoryLimit)
            }
            TrajectoryHistoryProjection::Supported(history)
                if history.iter().any(|atom| {
                    !atom.fits_limits(
                        self.config.max_blocks_per_atom,
                        self.config.max_structural_bytes_per_atom,
                    )
                }) =>
            {
                singleton_assignment(projected.action_id, LlmTrajectoryStartReason::HistoryLimit)
            }
            TrajectoryHistoryProjection::Supported(history) if history.is_empty() => {
                singleton_assignment(projected.action_id, LlmTrajectoryStartReason::Unspecified)
            }
            TrajectoryHistoryProjection::Supported(history) => {
                state.classify_supported(projected.action_id, &history, observed_at, &self.config)
            }
        };
        state.remember_assignment(assignment.clone(), self.config.assignment_cache_capacity());
        TrajectoryClassification::Assigned(assignment)
    }

    pub(in crate::llm_pipeline) fn classify_failure(
        &mut self,
        action_id: String,
    ) -> TrajectoryAssignment {
        singleton_assignment(action_id, LlmTrajectoryStartReason::ClassifierFailure)
    }

    pub(in crate::llm_pipeline) fn take_resolved(&mut self) -> Vec<TrajectoryAssignment> {
        std::mem::take(&mut self.resolved)
    }

    pub(in crate::llm_pipeline) fn register_provider_response(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        classifier_id: String,
        action_id: &str,
        provider_response_id: &str,
        observed_at: SystemTime,
    ) -> Vec<TrajectoryAssignment> {
        if provider_response_id.is_empty()
            || provider_response_id.len() > self.config.max_structural_bytes_per_atom
        {
            return Vec::new();
        }
        let scope = TrajectoryScope::new(trace_id, process, classifier_id);
        let Some(state) = self.scopes.get_mut(&scope) else {
            return Vec::new();
        };
        state.resolve_provider_response(action_id, provider_response_id, observed_at, &self.config)
    }

    pub(in crate::llm_pipeline) fn reject_parent_candidate(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        classifier_id: &str,
        action_id: &str,
    ) {
        let scope = TrajectoryScope::new(trace_id, process, classifier_id.to_string());
        let Some(state) = self.scopes.get_mut(&scope) else {
            return;
        };
        let Some(candidate_id) = state.candidate_by_action.get(action_id).copied() else {
            return;
        };
        state.remove_candidate(candidate_id);
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.scopes.retain(|scope, _| scope.trace_id != trace_id);
    }

    pub(in crate::llm_pipeline) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
    ) -> Vec<TrajectoryAssignment> {
        let maximum = self.config.assignment_cache_capacity();
        self.scopes
            .iter_mut()
            .filter(|(scope, _)| scope.trace_id == trace_id)
            .flat_map(|(_, state)| state.finalize_deferred(maximum))
            .collect()
    }
}

impl TrajectoryClassifierConfig {
    fn assignment_cache_capacity(&self) -> usize {
        let candidate_capacity = self
            .max_active_trajectories_per_scope
            .saturating_mul(self.max_candidate_nodes_per_trajectory);
        self.max_prefix_nodes_per_scope
            .max(candidate_capacity)
            .saturating_add(self.max_active_trajectories_per_scope)
    }
}

#[derive(Default)]
struct ScopeState {
    trie: PrefixTrie,
    candidates: BTreeMap<u64, Candidate>,
    candidate_by_action: HashMap<String, u64>,
    trajectories: BTreeMap<String, TrajectoryState>,
    assignments: BTreeMap<String, TrajectoryAssignment>,
    assignment_order: VecDeque<String>,
    provider_bindings: HashMap<String, ProviderBinding>,
    ambiguous_provider_ids: HashSet<String>,
    ambiguous_provider_order: VecDeque<String>,
    provider_binding_counts: HashMap<String, usize>,
    continued_actions: HashSet<String>,
    deferred_actions: HashMap<String, String>,
    deferred_by_provider: HashMap<String, BTreeMap<String, DeferredProviderRequest>>,
    deferred_expiry: BTreeSet<DeferredProviderExpiry>,
    pending_provider_responses: HashMap<String, PendingProviderResponse>,
    deferred_count: usize,
    next_candidate_id: u64,
}

impl ScopeState {
    fn remember_assignment(&mut self, assignment: TrajectoryAssignment, maximum: usize) {
        if self.assignments.contains_key(&assignment.action_id) {
            return;
        }
        self.assignment_order
            .push_back(assignment.action_id.clone());
        self.assignments
            .insert(assignment.action_id.clone(), assignment);
        while self.assignments.len() > maximum {
            let Some(action_id) = self.assignment_order.pop_front() else {
                break;
            };
            self.assignments.remove(&action_id);
        }
    }

    fn classify_supported(
        &mut self,
        action_id: String,
        history: &[HistoryAtom],
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) -> TrajectoryAssignment {
        let capacity_evicted = match self
            .make_path_capacity(history, config.max_prefix_nodes_per_scope)
        {
            Ok(evicted) => evicted,
            Err(()) => {
                return singleton_assignment(action_id, LlmTrajectoryStartReason::CapacityEviction);
            }
        };

        let path = self.trie.path(history);
        let exact_node = path.last().copied().unwrap_or(PrefixTrie::ROOT);
        let has_exact = path.len() == history.len()
            && self
                .trie
                .node(exact_node)
                .is_some_and(|node| !node.candidates.is_empty());
        if has_exact {
            let assignment = TrajectoryAssignment {
                action_id: action_id.clone(),
                trajectory_id: action_id,
                parent_action_id: None,
                forked_from_action_id: None,
                position: 0,
                transition: LlmTrajectoryTransition::DuplicateRoot,
                start_reason: LlmTrajectoryStartReason::Unspecified,
                inference_version: INFERENCE_VERSION,
            };
            self.prepare_new_trajectory(observed_at, config);
            self.index_assignment(&assignment, history, observed_at, config);
            return assignment;
        }

        let parent = path
            .iter()
            .take(history.len())
            .rev()
            .find_map(|node_id| self.best_candidate(*node_id));
        let assignment = match parent {
            Some(parent) if !parent.has_continuation => {
                self.append_to_parent(action_id, &parent, observed_at)
            }
            Some(parent) => {
                let forked_from_action_id = parent.action_id.clone();
                if let Some(trajectory) = self.trajectories.get_mut(&parent.trajectory_id) {
                    trajectory.last_seen = observed_at;
                }
                let trajectory_id = action_id.clone();
                TrajectoryAssignment {
                    action_id,
                    trajectory_id,
                    parent_action_id: None,
                    forked_from_action_id: Some(forked_from_action_id),
                    position: 0,
                    transition: LlmTrajectoryTransition::ForkRoot,
                    start_reason: LlmTrajectoryStartReason::Unspecified,
                    inference_version: INFERENCE_VERSION,
                }
            }
            None => {
                let trajectory_id = action_id.clone();
                TrajectoryAssignment {
                    action_id,
                    trajectory_id,
                    parent_action_id: None,
                    forked_from_action_id: None,
                    position: 0,
                    transition: LlmTrajectoryTransition::Root,
                    start_reason: if capacity_evicted {
                        LlmTrajectoryStartReason::CapacityEviction
                    } else {
                        LlmTrajectoryStartReason::Unspecified
                    },
                    inference_version: INFERENCE_VERSION,
                }
            }
        };

        if assignment.transition != LlmTrajectoryTransition::Append {
            self.prepare_new_trajectory(observed_at, config);
        }
        if self
            .make_path_capacity(history, config.max_prefix_nodes_per_scope)
            .is_ok()
        {
            self.index_assignment(&assignment, history, observed_at, config);
        }
        assignment
    }

    fn append_to_parent(
        &mut self,
        action_id: String,
        parent: &CandidateSelection,
        observed_at: SystemTime,
    ) -> TrajectoryAssignment {
        if let Some(candidate) = self.candidates.get_mut(&parent.id) {
            candidate.has_continuation = true;
        }
        self.continued_actions.insert(parent.action_id.clone());
        if let Some(trajectory) = self.trajectories.get_mut(&parent.trajectory_id) {
            trajectory.last_seen = observed_at;
        }
        TrajectoryAssignment {
            action_id,
            trajectory_id: parent.trajectory_id.clone(),
            parent_action_id: Some(parent.action_id.clone()),
            forked_from_action_id: None,
            position: parent.position.saturating_add(1),
            transition: LlmTrajectoryTransition::Append,
            start_reason: LlmTrajectoryStartReason::Unspecified,
            inference_version: INFERENCE_VERSION,
        }
    }

    fn best_candidate(&self, node_id: usize) -> Option<CandidateSelection> {
        self.trie
            .node(node_id)?
            .candidates
            .iter()
            .filter_map(|id| self.candidates.get(id))
            .min_by_key(|candidate| {
                (
                    candidate.has_continuation,
                    candidate.trajectory_id.as_str(),
                    candidate.action_id.as_str(),
                )
            })
            .map(CandidateSelection::from)
    }

    fn prepare_new_trajectory(
        &mut self,
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) {
        while self.trajectories.len() >= config.max_active_trajectories_per_scope {
            let Some(oldest) = self.oldest_trajectory() else {
                break;
            };
            self.remove_trajectory(&oldest);
        }
        self.expire_idle(observed_at, config.idle_ttl);
    }

    fn index_assignment(
        &mut self,
        assignment: &TrajectoryAssignment,
        history: &[HistoryAtom],
        observed_at: SystemTime,
        config: &TrajectoryClassifierConfig,
    ) {
        let Some(node_id) = self.trie.ensure_path(history) else {
            return;
        };
        let candidate_id = self.next_candidate_id;
        self.next_candidate_id = self.next_candidate_id.wrapping_add(1);
        self.candidates.insert(
            candidate_id,
            Candidate {
                node_id,
                action_id: assignment.action_id.clone(),
                trajectory_id: assignment.trajectory_id.clone(),
                position: assignment.position,
                has_continuation: false,
                observed_at,
                insertion_order: candidate_id,
            },
        );
        self.candidate_by_action
            .insert(assignment.action_id.clone(), candidate_id);
        let Some(node) = self.trie.node_mut(node_id) else {
            self.candidates.remove(&candidate_id);
            return;
        };
        node.candidates.insert(candidate_id);
        let expired = {
            let trajectory = self
                .trajectories
                .entry(assignment.trajectory_id.clone())
                .or_insert_with(|| TrajectoryState {
                    candidates: VecDeque::new(),
                    provider_response_ids: VecDeque::new(),
                    last_seen: observed_at,
                    insertion_order: candidate_id,
                });
            trajectory.last_seen = observed_at;
            trajectory.candidates.push_back(candidate_id);
            let mut expired = Vec::new();
            while trajectory.candidates.len() > config.max_candidate_nodes_per_trajectory {
                let Some(candidate_id) = trajectory.candidates.pop_front() else {
                    break;
                };
                expired.push(candidate_id);
            }
            expired
        };
        for candidate_id in expired {
            self.remove_candidate(candidate_id);
        }
    }

    fn make_path_capacity(&mut self, history: &[HistoryAtom], maximum: usize) -> Result<bool, ()> {
        if history.len() > maximum {
            return Err(());
        }
        let mut evicted = false;
        loop {
            let missing = self.trie.missing_nodes(history);
            if self.trie.non_root_node_count().saturating_add(missing) <= maximum {
                return Ok(evicted);
            }
            let Some(candidate_id) = self.oldest_candidate() else {
                return Err(());
            };
            self.remove_candidate(candidate_id);
            evicted = true;
        }
    }

    fn expire_idle(&mut self, observed_at: SystemTime, ttl: Duration) {
        let expired = self
            .trajectories
            .iter()
            .filter(|(_, trajectory)| {
                observed_at
                    .duration_since(trajectory.last_seen)
                    .is_ok_and(|idle| idle >= ttl)
            })
            .map(|(trajectory_id, _)| trajectory_id.clone())
            .collect::<Vec<_>>();
        for trajectory_id in expired {
            self.remove_trajectory(&trajectory_id);
        }
    }

    fn oldest_trajectory(&self) -> Option<String> {
        self.trajectories
            .iter()
            .min_by_key(|(trajectory_id, trajectory)| {
                (
                    trajectory.last_seen,
                    trajectory.insertion_order,
                    trajectory_id.as_str(),
                )
            })
            .map(|(trajectory_id, _)| trajectory_id.clone())
    }

    fn oldest_candidate(&self) -> Option<u64> {
        self.candidates
            .iter()
            .min_by_key(|(_, candidate)| (candidate.observed_at, candidate.insertion_order))
            .map(|(candidate_id, _)| *candidate_id)
    }

    fn remove_trajectory(&mut self, trajectory_id: &str) {
        let Some(trajectory) = self.trajectories.remove(trajectory_id) else {
            return;
        };
        for provider_response_id in trajectory.provider_response_ids {
            self.remove_provider_binding(&provider_response_id);
        }
        for candidate_id in trajectory.candidates {
            self.remove_candidate(candidate_id);
        }
    }

    fn remove_candidate(&mut self, candidate_id: u64) {
        let Some(candidate) = self.candidates.remove(&candidate_id) else {
            return;
        };
        self.candidate_by_action.remove(&candidate.action_id);
        let action_has_provider_binding = self
            .provider_binding_counts
            .get(&candidate.action_id)
            .is_some_and(|count| *count != 0);
        if !action_has_provider_binding {
            self.continued_actions.remove(&candidate.action_id);
        }
        if let Some(node) = self.trie.node_mut(candidate.node_id) {
            node.candidates.remove(&candidate_id);
        }
        let mut remove_trajectory = false;
        if let Some(trajectory) = self.trajectories.get_mut(&candidate.trajectory_id) {
            trajectory
                .candidates
                .retain(|candidate| *candidate != candidate_id);
            remove_trajectory =
                trajectory.candidates.is_empty() && trajectory.provider_response_ids.is_empty();
        }
        if remove_trajectory {
            self.trajectories.remove(&candidate.trajectory_id);
        }
        self.trie.prune(candidate.node_id);
    }
}

#[derive(Clone)]
struct CandidateSelection {
    id: u64,
    action_id: String,
    trajectory_id: String,
    position: u32,
    has_continuation: bool,
}

impl From<&Candidate> for CandidateSelection {
    fn from(candidate: &Candidate) -> Self {
        Self {
            id: candidate.insertion_order,
            action_id: candidate.action_id.clone(),
            trajectory_id: candidate.trajectory_id.clone(),
            position: candidate.position,
            has_continuation: candidate.has_continuation,
        }
    }
}

struct Candidate {
    node_id: usize,
    action_id: String,
    trajectory_id: String,
    position: u32,
    has_continuation: bool,
    observed_at: SystemTime,
    insertion_order: u64,
}

struct TrajectoryState {
    candidates: VecDeque<u64>,
    provider_response_ids: VecDeque<String>,
    last_seen: SystemTime,
    insertion_order: u64,
}

#[derive(Clone)]
enum ProviderBinding {
    Bound(ProviderParent),
}

#[derive(Clone)]
struct ProviderParent {
    action_id: String,
    trajectory_id: String,
    position: u32,
    observed_at: SystemTime,
}

struct DeferredProviderRequest {
    action_id: String,
    observed_at: SystemTime,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct DeferredProviderExpiry {
    observed_at: SystemTime,
    action_id: String,
}

struct PendingProviderResponse {
    provider_response_id: String,
    observed_at: SystemTime,
}

struct PendingProviderRegistration {
    action_id: String,
    provider_response_id: String,
    observed_at: SystemTime,
}

fn singleton_assignment(
    action_id: String,
    start_reason: LlmTrajectoryStartReason,
) -> TrajectoryAssignment {
    TrajectoryAssignment {
        trajectory_id: action_id.clone(),
        action_id,
        parent_action_id: None,
        forked_from_action_id: None,
        position: 0,
        transition: LlmTrajectoryTransition::Root,
        start_reason,
        inference_version: INFERENCE_VERSION,
    }
}

impl TrajectoryAssignment {
    pub(in crate::llm_pipeline) fn lineage(&self, trace_id: TraceId) -> LlmRequestLineageWrite {
        LlmRequestLineageWrite {
            trace_id,
            action_id: self.action_id.clone(),
            trajectory_id: self.trajectory_id.clone(),
            parent_action_id: self.parent_action_id.clone(),
            forked_from_action_id: self.forked_from_action_id.clone(),
            trajectory_position: self.position,
            transition: self.transition,
            start_reason: self.start_reason,
            inference_version: self.inference_version,
        }
    }
}

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, SystemTime};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use crate::llm_pipeline::projection::trajectory::TrajectoryAssignment;

mod carrier;
mod choice;
pub(super) use carrier::PreparedCarrierSnapshot;
pub(super) use choice::{AdoptionChoices, CandidateSource, PairEvidence, PairStatus};
mod entries;
mod evidence;
use entries::RetainedCall;
pub(super) use entries::{
    CompactionCandidate, CompletedCallHead, CompletedRequestHead, PendingCall, PredecessorHead,
    RetiredCandidate,
};
mod pending;
mod pools;
mod response;
pub(super) use evidence::{
    ArtifactContentMatch, ArtifactFlow, ArtifactFlowObservation, CandidateDetection,
    CompactConfirmation, ContextShape, DetectionOutcome, DetectorKind, DetectorMatch,
    PendingAdoption, ProtocolIntent, RequestHistoryWitness,
};
pub(super) use pools::AdmissionId;
use pools::{CompactionCandidatePool, PendingAdoptionIndex};
pub(super) use response::{
    JsonTextArtifact, ResponseFingerprints, ResponseTextWitness, ResponseWitnessObservation,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct CompactScopeKey {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
    pub(super) classifier_id: String,
}

pub(super) struct CompactStateStore {
    max_scopes_per_trace: usize,
    scopes: BTreeMap<CompactScopeKey, CompactScopeState>,
    scope_order: BTreeMap<TraceId, VecDeque<CompactScopeKey>>,
    pending_index: BTreeMap<(TraceId, String), CompactScopeKey>,
    dirty_adoptions: BTreeSet<(CompactScopeKey, String)>,
}

#[derive(Default)]
pub(super) struct CompactScopeState {
    pub(super) open_calls: BTreeMap<String, PendingCall>,
    pub(super) pending_order: VecDeque<String>,
    pub(super) retained_calls: BTreeMap<String, RetainedCall>,
    pub(super) latest_completed: Option<PredecessorHead>,
    pub(super) candidates: CompactionCandidatePool,
    pending_adoptions: PendingAdoptionIndex,
    next_admission: AdmissionId,
}

pub(super) struct ScopeAdmission {
    pub(super) evicted: Option<(CompactScopeKey, usize)>,
}

impl CompactStateStore {
    pub(super) fn has_waiting_adoption(
        &self,
        scope: &CompactScopeKey,
        candidate: &CompactionCandidate,
        observed_at: SystemTime,
    ) -> bool {
        // Explicit matching only uses the current scope, checked by its owner.
        if candidate.detector == DetectorKind::ExplicitProtocol {
            return false;
        }
        let start = CompactScopeKey {
            trace_id: scope.trace_id,
            process: scope.process,
            classifier_id: String::new(),
        };
        self.scopes
            .range(start..)
            .take_while(|(key, _)| key.trace_id == scope.trace_id && key.process == scope.process)
            .any(|(_, state)| state.has_waiting_adoption(candidate, observed_at))
    }

    pub(super) fn reject_candidate(
        &mut self,
        scope: &CompactScopeKey,
        candidate: &CompactionCandidate,
        observed_at: SystemTime,
    ) {
        if candidate.detector == DetectorKind::ArtifactFlow {
            for key in self.process_scope_keys(scope.trace_id, scope.process) {
                if let Some(state) = self.scopes.get_mut(&key) {
                    state.reject_candidate(candidate, observed_at);
                }
            }
        }
    }

    pub(super) fn new(max_scopes_per_trace: usize) -> Self {
        Self {
            max_scopes_per_trace,
            scopes: BTreeMap::new(),
            scope_order: BTreeMap::new(),
            pending_index: BTreeMap::new(),
            dirty_adoptions: BTreeSet::new(),
        }
    }

    pub(super) fn admit(&mut self, key: &CompactScopeKey) -> ScopeAdmission {
        if self.scopes.contains_key(key) {
            return ScopeAdmission { evicted: None };
        }
        let queue = self.scope_order.entry(key.trace_id).or_default();
        let evicted_key = if queue.len() >= self.max_scopes_per_trace {
            queue.pop_front()
        } else {
            None
        };
        queue.push_back(key.clone());
        let mut eviction = None;
        if let Some(evicted) = evicted_key
            && let Some(state) = self.scopes.remove(&evicted)
        {
            self.pending_index
                .retain(|_, pending_scope| pending_scope != &evicted);
            self.dirty_adoptions.retain(|(scope, _)| scope != &evicted);
            for candidate in state.candidates.values() {
                self.retire_candidate(&evicted, &RetiredCandidate::from(candidate));
            }
            eviction = Some((evicted, state.entry_count()));
        }
        self.scopes
            .insert(key.clone(), CompactScopeState::default());
        ScopeAdmission { evicted: eviction }
    }

    pub(super) fn take(&mut self, key: &CompactScopeKey) -> Option<CompactScopeState> {
        self.scopes.remove(key)
    }

    pub(super) fn put(&mut self, key: CompactScopeKey, mut state: CompactScopeState) {
        for candidate in state.candidates.take_removed() {
            for action_id in state.retire_candidate(&candidate) {
                self.mark_dirty(&key, action_id);
            }
            self.retire_candidate(&key, &candidate);
        }
        for action_id in state.prune_retained() {
            self.remove_pending_index(key.trace_id, &action_id);
        }
        self.scopes.insert(key, state);
    }

    pub(super) fn process_candidates<'a>(
        &'a self,
        key: &'a CompactScopeKey,
        started_at: SystemTime,
        ttl: Duration,
    ) -> impl Iterator<Item = (&'a CompactScopeKey, AdmissionId, &'a CompactionCandidate)> {
        let start = CompactScopeKey {
            trace_id: key.trace_id,
            process: key.process,
            classifier_id: String::new(),
        };
        self.scopes
            .range(start..)
            .take_while(move |(scope, _)| {
                scope.trace_id == key.trace_id && scope.process == key.process
            })
            .flat_map(move |(scope, state)| {
                state
                    .candidates
                    .for_adoption(started_at, ttl)
                    .into_iter()
                    .map(move |(id, candidate)| (scope, id, candidate))
            })
    }

    pub(super) fn remove_candidate(
        &mut self,
        key: &CompactScopeKey,
        id: AdmissionId,
    ) -> Option<CompactionCandidate> {
        let state = self.scopes.get_mut(key)?;
        let candidate = state.candidates.remove(id);
        let released = state.prune_retained();
        let removed = state.candidates.take_removed();
        for action_id in released {
            self.remove_pending_index(key.trace_id, &action_id);
        }
        for retired in removed {
            self.retire_candidate(key, &retired);
        }
        candidate
    }

    pub(super) fn candidate(
        &self,
        key: &CompactScopeKey,
        id: AdmissionId,
    ) -> Option<&CompactionCandidate> {
        self.scopes.get(key)?.candidates.get(id)
    }

    pub(super) fn refine_trajectory(
        &mut self,
        key: &CompactScopeKey,
        assignment: &TrajectoryAssignment,
    ) -> bool {
        self.scopes
            .get_mut(key)
            .is_some_and(|state| state.refine_trajectory(&assignment.action_id, assignment))
    }

    pub(super) fn candidate_for_action(
        &self,
        key: &CompactScopeKey,
        action_id: &str,
    ) -> Option<(AdmissionId, &CompactionCandidate)> {
        self.scopes.get(key)?.candidates.by_action(action_id)
    }

    pub(super) fn mark_dirty(&mut self, key: &CompactScopeKey, action_id: String) {
        self.dirty_adoptions.insert((key.clone(), action_id));
    }

    pub(super) fn take_dirty(&mut self) -> BTreeSet<(CompactScopeKey, String)> {
        std::mem::take(&mut self.dirty_adoptions)
    }

    fn retire_candidate(&mut self, owner: &CompactScopeKey, candidate: &RetiredCandidate) {
        for key in self.process_scope_keys(owner.trace_id, owner.process) {
            if let Some(state) = self.scopes.get_mut(&key) {
                for action_id in state.retire_candidate(candidate) {
                    self.dirty_adoptions.insert((key.clone(), action_id));
                }
            }
        }
    }

    pub(super) fn process_scope_keys(
        &self,
        trace_id: TraceId,
        process: ProcessIdentity,
    ) -> Vec<CompactScopeKey> {
        let start = CompactScopeKey {
            trace_id,
            process,
            classifier_id: String::new(),
        };
        self.scopes
            .range(start..)
            .take_while(|(key, _)| key.trace_id == trace_id && key.process == process)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub(super) fn index_pending(&mut self, key: &CompactScopeKey, action_id: &str) {
        self.pending_index
            .insert((key.trace_id, action_id.to_string()), key.clone());
    }

    pub(super) fn remove_pending_index(&mut self, trace_id: TraceId, action_id: &str) {
        self.pending_index
            .remove(&(trace_id, action_id.to_string()));
    }

    pub(super) fn pending_scope(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Option<CompactScopeKey> {
        self.pending_index
            .get(&(trace_id, action_id.to_string()))
            .cloned()
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.dirty_adoptions
            .retain(|(key, _)| key.trace_id != trace_id);
        if let Some(keys) = self.scope_order.remove(&trace_id) {
            for key in keys {
                self.scopes.remove(&key);
            }
        }
        self.pending_index
            .retain(|(candidate, _), _| *candidate != trace_id);
    }
}

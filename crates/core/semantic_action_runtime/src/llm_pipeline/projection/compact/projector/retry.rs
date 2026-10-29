use super::*;

pub(super) struct CandidateTrigger {
    pub(super) scope: CompactScopeKey,
    pub(super) admission_id: super::super::state::AdmissionId,
    pub(super) completed_at: SystemTime,
    pub(super) expires_at: SystemTime,
    pub(super) detector: DetectorKind,
}

impl CandidateTrigger {
    pub(super) fn new(
        scope: CompactScopeKey,
        admission_id: super::super::state::AdmissionId,
        candidate: &CompactionCandidate,
    ) -> Option<Self> {
        Some(Self {
            scope,
            admission_id,
            completed_at: candidate.implementation.response_end_time?,
            expires_at: candidate.expires_at,
            detector: candidate.detector,
        })
    }
}

impl ContextCompactProjector {
    /// A new or once-refined B updates only its own pairs in the indexed C time range.
    pub(super) fn retry_process(
        &mut self,
        trigger: &CandidateTrigger,
        observed_at: SystemTime,
        output: &mut CompactProjectionBatch,
    ) {
        let keys = if trigger.detector == DetectorKind::ArtifactFlow {
            self.states
                .process_scope_keys(trigger.scope.trace_id, trigger.scope.process)
        } else {
            vec![trigger.scope.clone()]
        };
        for key in keys {
            let Some(mut state) = self.states.take(&key) else {
                continue;
            };
            self.expire_scope(&key, &mut state, observed_at, None);
            for action_id in
                state.adoption_request_ids(Some(trigger.completed_at), Some(trigger.expires_at))
            {
                let Some((mut pending, completed)) = state.take_pending(&action_id) else {
                    continue;
                };
                if let Some(adoption) = pending
                    .adoption
                    .as_mut()
                    .filter(|adoption| adoption.can_wait(observed_at))
                {
                    let candidate = if key == trigger.scope {
                        state.candidates.get(trigger.admission_id)
                    } else {
                        self.states.candidate(&trigger.scope, trigger.admission_id)
                    };
                    if let Some(candidate) = candidate {
                        let changed = self.update_pair(
                            &trigger.scope,
                            trigger.admission_id,
                            candidate,
                            &pending.request,
                            pending.trajectory.as_ref(),
                            adoption,
                            output,
                        );
                        if changed {
                            self.states.mark_dirty(&key, action_id.clone());
                        }
                    }
                }
                self.restore_pending(&key, &mut state, pending, completed);
            }
            self.states.put(key, state);
        }
    }

    pub(super) fn initialize_pending(
        &mut self,
        key: &CompactScopeKey,
        action_ids: &[String],
        observed_at: SystemTime,
        output: &mut CompactProjectionBatch,
    ) {
        let Some(mut state) = self.states.take(key) else {
            return;
        };
        self.expire_scope(key, &mut state, observed_at, None);
        for action_id in action_ids {
            if let Some((mut pending, completed)) = state.take_pending(action_id) {
                if let Some(adoption) = pending
                    .adoption
                    .as_mut()
                    .filter(|adoption| adoption.can_wait(observed_at))
                {
                    self.initialize_adoption(
                        &state,
                        key,
                        &pending.request,
                        pending.trajectory.as_ref(),
                        adoption,
                        output,
                    );
                    self.states.mark_dirty(key, action_id.to_string());
                }
                self.restore_pending(key, &mut state, pending, completed);
            }
        }
        self.states.put(key.clone(), state);
    }

    /// All fact/pair updates in an input batch finish before any source is consumed.
    pub(super) fn commit_adoptions(
        &mut self,
        observed_at: SystemTime,
        output: &mut CompactProjectionBatch,
    ) {
        loop {
            let dirty = self.states.take_dirty();
            if dirty.is_empty() {
                break;
            }
            let mut scopes = BTreeMap::<_, Vec<_>>::new();
            for (key, action_id) in dirty {
                scopes.entry(key).or_default().push(action_id);
            }
            for (key, action_ids) in scopes {
                let Some(mut state) = self.states.take(&key) else {
                    continue;
                };
                for action_id in action_ids {
                    if let Some((mut pending, completed)) = state.take_pending(&action_id) {
                        let mut adoption = pending.adoption.take();
                        let outcome = self.confirm_adoption(
                            &mut state,
                            &key,
                            &pending.request,
                            pending.trajectory.as_ref(),
                            adoption.as_mut(),
                            observed_at,
                            output,
                        );
                        if !matches!(outcome, DetectionOutcome::Confirmed) {
                            pending.adoption =
                                adoption.filter(|adoption| adoption.can_wait(observed_at));
                        }
                        self.restore_pending(&key, &mut state, pending, completed);
                    }
                }
                self.states.put(key, state);
            }
        }
    }

    fn restore_pending(
        &mut self,
        key: &CompactScopeKey,
        state: &mut CompactScopeState,
        pending: PendingCall,
        completed: bool,
    ) {
        let action_id = pending.request.action_id.clone();
        let retain = state.needs_retention(&pending, completed, self.config.trajectory_enabled);
        state.put_pending(pending, completed, retain);
        if retain {
            self.states.index_pending(key, &action_id);
        } else {
            self.states.remove_pending_index(key.trace_id, &action_id);
        }
    }

    pub(super) fn expire_scope(
        &mut self,
        key: &CompactScopeKey,
        state: &mut CompactScopeState,
        observed_at: SystemTime,
        incoming: Option<(&SemanticAction, Option<&TrajectoryFact>)>,
    ) -> usize {
        let before = state.candidates.len();
        let released = state.expire_adoptions(observed_at, self.config.pending_adoptions.ttl);
        for id in state.candidates.expired_ids(observed_at) {
            let Some(candidate) = state.candidates.get(id) else {
                continue;
            };
            if incoming.is_some_and(|(request, current)| {
                candidate.applies_to(request, current, &self.gate)
            }) {
                continue;
            }
            if !state.has_waiting_adoption(candidate, observed_at)
                && !self
                    .states
                    .has_waiting_adoption(key, candidate, observed_at)
            {
                state.candidates.remove(id);
            }
        }
        let expired = before
            .saturating_sub(state.candidates.len())
            .saturating_add(released.len());
        for action_id in released {
            if let Some((pending, completed)) = state.take_pending(&action_id) {
                self.restore_pending(key, state, pending, completed);
            }
        }
        expired
    }
}

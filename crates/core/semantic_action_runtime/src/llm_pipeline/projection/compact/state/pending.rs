use super::*;

impl CompactScopeState {
    pub(in crate::llm_pipeline::projection::compact) fn retire_candidate(
        &mut self,
        candidate: &RetiredCandidate,
    ) -> Vec<String> {
        let mut dirty = Vec::new();
        for pending in self.open_calls.values_mut().chain(
            self.retained_calls
                .values_mut()
                .map(|value| &mut value.pending),
        ) {
            let Some(adoption) = pending.adoption.as_mut() else {
                continue;
            };
            let Some(pair) = adoption.matches.remove(&candidate.action_id) else {
                continue;
            };
            if candidate.disproves(pending.trajectory.as_ref()) {
                adoption.choices.group(pair.detector).exclude(pair.status);
            } else {
                adoption.choices.group(pair.detector).discard(pair.status);
            }
            if pair.status != PairStatus::NoMatch {
                dirty.push(pending.request.action_id.clone());
            }
        }
        dirty
    }
    pub(in crate::llm_pipeline::projection::compact) fn allocate_admission(
        &mut self,
    ) -> Option<AdmissionId> {
        let next = self.next_admission.checked_add(1)?;
        let id = self.next_admission;
        self.next_admission = next;
        Some(id)
    }

    pub(in crate::llm_pipeline::projection::compact) fn insert_request(
        &mut self,
        pending: PendingCall,
    ) {
        self.pending_adoptions.update(&pending);
        self.pending_order
            .push_back(pending.request.action_id.clone());
        self.open_calls
            .insert(pending.request.action_id.clone(), pending);
    }

    pub(in crate::llm_pipeline::projection::compact) fn take_open(
        &mut self,
        action_id: &str,
    ) -> Option<PendingCall> {
        let pending = self.open_calls.remove(action_id)?;
        self.pending_adoptions.remove(&pending);
        Some(pending)
    }

    pub(in crate::llm_pipeline::projection::compact) fn adoption_request_ids(
        &self,
        from: Option<SystemTime>,
        until: Option<SystemTime>,
    ) -> Vec<String> {
        self.pending_adoptions.ids(from, until)
    }

    pub(in crate::llm_pipeline::projection::compact) fn needs_retention(
        &self,
        pending: &PendingCall,
        retained: bool,
        trajectory_enabled: bool,
    ) -> bool {
        pending.has_pending_adoption()
            || (trajectory_enabled
                && pending.trajectory.is_none()
                && (!retained
                    || self
                        .candidates
                        .by_action(&pending.request.action_id)
                        .is_some()))
    }

    pub(in crate::llm_pipeline::projection::compact) fn make_adoption_room(
        &mut self,
        capacity: usize,
    ) -> Vec<String> {
        let pending_adoptions = self.pending_adoptions.len();
        let mut remaining = pending_adoptions.saturating_add(1).saturating_sub(capacity);
        let mut evicted = Vec::new();
        for action_id in &self.pending_order {
            if remaining == 0 {
                break;
            }
            let pending = self.open_calls.get_mut(action_id).or_else(|| {
                self.retained_calls
                    .get_mut(action_id)
                    .map(|value| &mut value.pending)
            });
            if let Some(pending) = pending
                && pending.adoption.take().is_some()
            {
                self.pending_adoptions.remove(pending);
                remaining -= 1;
                evicted.push(action_id.clone());
            }
        }
        evicted
    }

    pub(super) fn prune_retained(&mut self) -> Vec<String> {
        let mut released = Vec::new();
        self.retained_calls.retain(|action_id, retained| {
            let keep = retained.pending.has_pending_adoption()
                || (retained.pending.trajectory.is_none()
                    && self.candidates.by_action(action_id).is_some());
            if !keep {
                self.pending_adoptions.remove(&retained.pending);
                released.push(action_id.clone());
            }
            keep
        });
        if !released.is_empty() {
            self.pending_order.retain(|id| {
                self.open_calls.contains_key(id) || self.retained_calls.contains_key(id)
            });
        }
        released
    }

    pub(in crate::llm_pipeline::projection::compact) fn has_waiting_adoption(
        &self,
        candidate: &CompactionCandidate,
        observed_at: SystemTime,
    ) -> bool {
        let Some(completed_at) = candidate.implementation.response_end_time else {
            return false;
        };
        self.pending_adoptions
            .has_in_range(completed_at, candidate.expires_at, |action_id| {
                self.open_calls
                    .get(action_id)
                    .or_else(|| self.retained_calls.get(action_id).map(|call| &call.pending))
                    .is_some_and(|pending| pending.waits_for_candidate(candidate, observed_at))
            })
    }

    pub(in crate::llm_pipeline::projection::compact) fn reject_candidate(
        &mut self,
        candidate: &CompactionCandidate,
        observed_at: SystemTime,
    ) {
        for pending in self.open_calls.values_mut().chain(
            self.retained_calls
                .values_mut()
                .map(|value| &mut value.pending),
        ) {
            if pending.waits_for_candidate(candidate, observed_at)
                && let Some(adoption) = pending.adoption.as_mut()
            {
                // A late source can compete with an existing source even before
                // C has a trajectory and initializes its pairs.
                adoption
                    .choices
                    .group(candidate.detector)
                    .discard(PairStatus::Pending);
            }
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn expire_adoptions(
        &mut self,
        observed_at: SystemTime,
        ttl: Duration,
    ) -> Vec<String> {
        let mut released = Vec::new();
        // If now + TTL is representable, every earlier start also has a normal
        // deadline. Otherwise include checked-add fallback deadlines and let
        // each adoption's actual expiry decide.
        let cutoff = if observed_at.checked_add(ttl).is_some() {
            let Some(cutoff) = observed_at.checked_sub(ttl) else {
                return released;
            };
            cutoff
        } else {
            observed_at
        };
        for id in self.pending_adoptions.started_by(cutoff) {
            let pending = self.open_calls.get_mut(&id).or_else(|| {
                self.retained_calls
                    .get_mut(&id)
                    .map(|value| &mut value.pending)
            });
            let Some(pending) = pending else { continue };
            if pending
                .adoption
                .as_ref()
                .is_some_and(|adoption| adoption.expires_at <= observed_at)
            {
                pending.adoption = None;
                self.pending_adoptions.remove(pending);
                released.push(pending.request.action_id.clone());
            }
        }
        released
    }

    pub(in crate::llm_pipeline::projection::compact) fn take_pending(
        &mut self,
        action_id: &str,
    ) -> Option<(PendingCall, bool)> {
        if let Some(completed) = self.retained_calls.remove(action_id) {
            self.pending_adoptions.remove(&completed.pending);
            Some((completed.pending, true))
        } else {
            self.take_open(action_id).map(|pending| (pending, false))
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn put_pending(
        &mut self,
        pending: PendingCall,
        completed: bool,
        retain_completed: bool,
    ) {
        let action_id = pending.request.action_id.clone();
        if !completed {
            self.pending_adoptions.update(&pending);
            self.open_calls.insert(action_id, pending);
        } else if retain_completed {
            self.pending_adoptions.update(&pending);
            self.retained_calls
                .insert(action_id, RetainedCall { pending });
        } else {
            self.pending_adoptions.remove(&pending);
            self.pending_order
                .retain(|candidate| candidate != &action_id);
        }
    }

    pub(super) fn entry_count(&self) -> usize {
        self.open_calls
            .len()
            .saturating_add(self.retained_calls.len())
            .saturating_add(self.candidates.len())
            .saturating_add(usize::from(self.latest_completed.is_some()))
    }

    pub(in crate::llm_pipeline::projection::compact) fn refine_trajectory(
        &mut self,
        action_id: &str,
        assignment: &TrajectoryAssignment,
    ) -> bool {
        let pending = self.open_calls.get_mut(action_id).or_else(|| {
            self.retained_calls
                .get_mut(action_id)
                .map(|retained| &mut retained.pending)
        });
        let Some(pending) = pending else { return false };
        if !pending.trajectory.resolve(assignment) {
            return false;
        }
        if pending
            .trajectory
            .as_ref()
            .is_some_and(|fact| !fact.allows_adoption())
        {
            pending.adoption = None;
            self.pending_adoptions.remove(pending);
        }
        true
    }
}

impl PendingCall {
    fn waits_for_candidate(
        &self,
        candidate: &CompactionCandidate,
        observed_at: SystemTime,
    ) -> bool {
        let Some(adoption) = self.adoption.as_ref() else {
            return false;
        };
        adoption.can_wait(observed_at)
            && adoption.supports(candidate.detector)
            && adoption.choices.can_resolve(candidate.detector)
            && (candidate.detector != DetectorKind::ArtifactFlow
                || candidate.implementation.history_source.is_some())
            && candidate.applies_to(
                &self.request,
                self.trajectory.as_ref(),
                &super::super::gate::TrajectoryBoundaryGate,
            )
            && !adoption
                .matches
                .get(&candidate.implementation.request.action_id)
                .is_some_and(|pair| {
                    pair.status == PairStatus::NoMatch
                        && matches!(pair.content, Some(ArtifactContentMatch::NoMatch))
                })
    }
}

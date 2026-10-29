use std::collections::{BTreeMap, BTreeSet};
use std::ops::{
    Bound::{Excluded, Included, Unbounded},
    RangeBounds,
};
use std::time::{Duration, SystemTime};

use super::{CompactionCandidate, PendingCall, RetiredCandidate};

/// Scope-local order; persisted request and call identities remain unchanged.
pub(in crate::llm_pipeline::projection::compact) type AdmissionId = u64;

#[derive(Default)]
pub(in crate::llm_pipeline::projection::compact) struct CompactionCandidatePool {
    entries: BTreeMap<AdmissionId, Box<CompactionCandidate>>,
    // With a fixed compaction candidate TTL, expiry order is completion order. Indexing the
    // actual deadline also handles checked-add overflow and expiry directly.
    by_expiry: BTreeSet<(SystemTime, AdmissionId)>,
    by_action: BTreeMap<String, AdmissionId>,
    removed: Vec<RetiredCandidate>,
}

impl CompactionCandidatePool {
    pub(in crate::llm_pipeline::projection::compact) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(in crate::llm_pipeline::projection::compact) fn insert(
        &mut self,
        id: AdmissionId,
        candidate: CompactionCandidate,
        capacity: usize,
        eviction: Option<AdmissionId>,
    ) -> Result<usize, CompactionCandidate> {
        let before = self.removed.len();
        if self.entries.len() >= capacity {
            self.retain(|candidate| candidate.is_eligible());
        }
        if self.entries.len() >= capacity
            && let Some(id) = eviction
        {
            self.remove(id);
        }
        if self.entries.len() >= capacity {
            return Err(candidate);
        }
        self.by_expiry.insert((candidate.expires_at, id));
        self.by_action
            .insert(candidate.implementation.request.action_id.clone(), id);
        self.entries.insert(id, Box::new(candidate));
        Ok(self.removed.len().saturating_sub(before))
    }

    pub(in crate::llm_pipeline::projection::compact) fn eviction_candidate(
        &self,
        mut can_reclaim: impl FnMut(&CompactionCandidate) -> bool,
    ) -> Option<AdmissionId> {
        self.entries.iter().find_map(|(&id, candidate)| {
            (!candidate.is_eligible() || can_reclaim(candidate)).then_some(id)
        })
    }

    pub(in crate::llm_pipeline::projection::compact) fn remove(
        &mut self,
        id: AdmissionId,
    ) -> Option<CompactionCandidate> {
        let candidate = self.entries.remove(&id)?;
        self.by_expiry.remove(&(candidate.expires_at, id));
        self.by_action
            .remove(&candidate.implementation.request.action_id);
        self.removed
            .push(RetiredCandidate::from(candidate.as_ref()));
        Some(*candidate)
    }

    pub(in crate::llm_pipeline::projection::compact) fn values(
        &self,
    ) -> impl Iterator<Item = &CompactionCandidate> {
        self.entries.values().map(Box::as_ref)
    }

    pub(in crate::llm_pipeline::projection::compact) fn retain(
        &mut self,
        mut keep: impl FnMut(&mut CompactionCandidate) -> bool,
    ) {
        let removed = self
            .entries
            .iter_mut()
            .filter_map(|(id, candidate)| (!keep(candidate.as_mut())).then_some(*id))
            .collect::<Vec<_>>();
        for id in removed {
            self.remove(id);
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn take_removed(
        &mut self,
    ) -> Vec<RetiredCandidate> {
        std::mem::take(&mut self.removed)
    }

    pub(in crate::llm_pipeline::projection::compact) fn by_action(
        &self,
        action_id: &str,
    ) -> Option<(AdmissionId, &CompactionCandidate)> {
        let id = *self.by_action.get(action_id)?;
        Some((id, self.get(id)?))
    }

    pub(in crate::llm_pipeline::projection::compact) fn for_adoption(
        &self,
        started_at: SystemTime,
        ttl: Duration,
    ) -> Vec<(AdmissionId, &CompactionCandidate)> {
        let expires_by = started_at.checked_add(ttl);
        // A fully covered pool already has the required arbitration order;
        // avoid sorting and looking up every entry through the second index.
        if self
            .by_expiry
            .first()
            .is_some_and(|(expiry, _)| *expiry > started_at)
            && self
                .by_expiry
                .last()
                .is_some_and(|(expiry, _)| expires_by.is_none_or(|end| *expiry <= end))
        {
            return self
                .entries
                .iter()
                .rev()
                .map(|(id, candidate)| (*id, candidate.as_ref()))
                .collect();
        }
        let upper = expires_by.map_or(Unbounded, |end| Included((end, AdmissionId::MAX)));
        let mut ids = self
            .by_expiry
            .range((Excluded((started_at, AdmissionId::MAX)), upper))
            .map(|(_, id)| *id)
            .collect::<Vec<_>>();
        ids.sort_unstable_by(|left, right| right.cmp(left));
        ids.into_iter()
            .filter_map(|id| {
                self.entries
                    .get(&id)
                    .map(|candidate| (id, candidate.as_ref()))
            })
            .collect()
    }

    pub(in crate::llm_pipeline::projection::compact) fn expired_ids(
        &self,
        observed_at: SystemTime,
    ) -> Vec<AdmissionId> {
        let mut ids = self
            .by_expiry
            .range(..=(observed_at, AdmissionId::MAX))
            .map(|(_, id)| *id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub(in crate::llm_pipeline::projection::compact) fn get(
        &self,
        id: AdmissionId,
    ) -> Option<&CompactionCandidate> {
        self.entries.get(&id).map(Box::as_ref)
    }
}

#[derive(Default)]
pub(super) struct PendingAdoptionIndex {
    by_start: BTreeMap<(SystemTime, AdmissionId), String>,
}

impl PendingAdoptionIndex {
    pub(super) fn has_in_range(
        &self,
        from: SystemTime,
        until: SystemTime,
        mut matches: impl FnMut(&str) -> bool,
    ) -> bool {
        from < until
            && self
                .by_start
                .range((Included((from, 0)), Excluded((until, 0))))
                .any(|(_, action_id)| matches(action_id))
    }
    pub(super) fn len(&self) -> usize {
        self.by_start.len()
    }

    pub(super) fn update(&mut self, pending: &PendingCall) {
        if pending.has_pending_adoption() {
            self.by_start.insert(
                (pending.request.start_time, pending.admission_id),
                pending.request.action_id.clone(),
            );
        } else {
            self.remove(pending);
        }
    }

    pub(super) fn remove(&mut self, pending: &PendingCall) {
        self.by_start
            .remove(&(pending.request.start_time, pending.admission_id));
    }

    pub(super) fn ids(&self, from: Option<SystemTime>, until: Option<SystemTime>) -> Vec<String> {
        let lower = from.map_or(Unbounded, |start| Included((start, 0)));
        let upper = until.map_or(Unbounded, |end| Excluded((end, 0)));
        self.ordered_ids((lower, upper))
    }

    pub(super) fn started_by(&self, observed_at: SystemTime) -> Vec<String> {
        self.ordered_ids(..=(observed_at, AdmissionId::MAX))
    }

    fn ordered_ids(&self, range: impl RangeBounds<(SystemTime, AdmissionId)>) -> Vec<String> {
        let mut hits = self
            .by_start
            .range(range)
            .map(|((_, order), id)| (*order, id))
            .collect::<Vec<_>>();
        hits.sort_unstable_by_key(|(order, _)| *order);
        hits.into_iter().map(|(_, id)| id.clone()).collect()
    }
}

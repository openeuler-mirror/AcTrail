//! Ordered, per-trace capacity index for projection-owned state.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Included, Unbounded};

use model_core::ids::TraceId;

pub(super) enum StateAdmission {
    Inserted,
    Existing,
    Evicted((TraceId, String)),
    SequenceExhausted,
}

/// Maintains insertion order without stale queue tokens. The primary state
/// remains in its owning map; this index only owns ordering and per-trace
/// counts, so insert/remove/oldest eviction are all O(log n).
pub(super) struct BoundedTraceIndex {
    limit_per_trace: usize,
    next_sequence: u64,
    positions: BTreeMap<(TraceId, String), u64>,
    order: BTreeSet<(TraceId, u64, String)>,
    counts: BTreeMap<TraceId, usize>,
}

impl BoundedTraceIndex {
    pub(super) fn new(limit_per_trace: usize) -> Self {
        Self {
            limit_per_trace,
            next_sequence: 0,
            positions: BTreeMap::new(),
            order: BTreeSet::new(),
            counts: BTreeMap::new(),
        }
    }

    pub(super) fn admit(&mut self, key: &(TraceId, String)) -> StateAdmission {
        if self.positions.contains_key(key) {
            return StateAdmission::Existing;
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            return StateAdmission::SequenceExhausted;
        };
        let evicted = (self.counts.get(&key.0).copied().unwrap_or_default()
            >= self.limit_per_trace)
            .then(|| self.oldest_for_trace(key.0))
            .flatten();
        if let Some(evicted) = &evicted {
            self.remove(evicted);
        }
        self.next_sequence = next_sequence;
        self.positions.insert(key.clone(), next_sequence);
        self.order.insert((key.0, next_sequence, key.1.clone()));
        *self.counts.entry(key.0).or_default() += 1;
        evicted.map_or(StateAdmission::Inserted, StateAdmission::Evicted)
    }

    pub(super) fn remove(&mut self, key: &(TraceId, String)) -> bool {
        let Some(sequence) = self.positions.remove(key) else {
            return false;
        };
        self.order.remove(&(key.0, sequence, key.1.clone()));
        if let Some(count) = self.counts.get_mut(&key.0) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.counts.remove(&key.0);
            }
        }
        true
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) -> Vec<(TraceId, String)> {
        let mut removed = Vec::new();
        while let Some(key) = self.oldest_for_trace(trace_id) {
            self.remove(&key);
            removed.push(key);
        }
        removed
    }

    fn oldest_for_trace(&self, trace_id: TraceId) -> Option<(TraceId, String)> {
        self.order
            .range((Included((trace_id, 0, String::new())), Unbounded))
            .next()
            .filter(|(candidate, _, _)| *candidate == trace_id)
            .map(|(candidate, _, action_id)| (*candidate, action_id.clone()))
    }
}

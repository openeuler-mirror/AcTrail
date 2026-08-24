//! Ordered ownership for bounded response-correlation bindings.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Included, Unbounded};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use super::LlmStreamKey;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct BindingOwner {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
}

struct BindingPosition {
    sequence: u64,
    owner: BindingOwner,
}

pub(in crate::llm_pipeline) enum BindingAdmission {
    Inserted,
    Existing,
    Evicted((TraceId, String)),
    SequenceExhausted,
}

pub(in crate::llm_pipeline) enum StreamAdmission {
    Inserted,
    Existing,
    Evicted(LlmStreamKey),
    SequenceExhausted,
}

pub(in crate::llm_pipeline) struct StreamOwnershipIndex {
    limit_per_trace: usize,
    next_sequence: u64,
    positions: BTreeMap<LlmStreamKey, u64>,
    order: BTreeMap<TraceId, BTreeMap<u64, LlmStreamKey>>,
    counts: BTreeMap<TraceId, usize>,
}

impl StreamOwnershipIndex {
    pub(in crate::llm_pipeline) fn new(limit_per_trace: usize) -> Self {
        Self {
            limit_per_trace,
            next_sequence: 0,
            positions: BTreeMap::new(),
            order: BTreeMap::new(),
            counts: BTreeMap::new(),
        }
    }

    pub(in crate::llm_pipeline) fn admit(&mut self, key: &LlmStreamKey) -> StreamAdmission {
        if self.positions.contains_key(key) {
            return StreamAdmission::Existing;
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            return StreamAdmission::SequenceExhausted;
        };
        let evicted = (self.counts.get(&key.trace_id).copied().unwrap_or_default()
            >= self.limit_per_trace)
            .then(|| self.oldest_for_trace(key.trace_id))
            .flatten();
        if let Some(evicted) = &evicted {
            self.remove(evicted);
        }
        self.next_sequence = next_sequence;
        self.positions.insert(key.clone(), next_sequence);
        self.order
            .entry(key.trace_id)
            .or_default()
            .insert(next_sequence, key.clone());
        *self.counts.entry(key.trace_id).or_default() += 1;
        evicted.map_or(StreamAdmission::Inserted, StreamAdmission::Evicted)
    }

    pub(in crate::llm_pipeline) fn remove(&mut self, key: &LlmStreamKey) -> bool {
        let Some(sequence) = self.positions.remove(key) else {
            return false;
        };
        if let Some(order) = self.order.get_mut(&key.trace_id) {
            order.remove(&sequence);
            if order.is_empty() {
                self.order.remove(&key.trace_id);
            }
        }
        if let Some(count) = self.counts.get_mut(&key.trace_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.counts.remove(&key.trace_id);
            }
        }
        true
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        while let Some(key) = self.oldest_for_trace(trace_id) {
            self.remove(&key);
        }
    }

    fn oldest_for_trace(&self, trace_id: TraceId) -> Option<LlmStreamKey> {
        self.order
            .get(&trace_id)
            .and_then(|order| order.first_key_value())
            .map(|(_, key)| key.clone())
    }
}

/// Keeps per-trace binding state bounded without stale FIFO tokens. The
/// binding payload remains in its owning map; this index owns only admission
/// order and counts.
pub(in crate::llm_pipeline) struct BindingOwnershipIndex {
    limit_per_trace: usize,
    next_sequence: u64,
    positions: BTreeMap<(TraceId, String), BindingPosition>,
    order: BTreeSet<(TraceId, u64, String)>,
    counts: BTreeMap<TraceId, usize>,
    owners: BTreeMap<BindingOwner, BTreeSet<(TraceId, String)>>,
}

impl BindingOwnershipIndex {
    pub(in crate::llm_pipeline) fn new(limit_per_trace: usize) -> Self {
        Self {
            limit_per_trace,
            next_sequence: 0,
            positions: BTreeMap::new(),
            order: BTreeSet::new(),
            counts: BTreeMap::new(),
            owners: BTreeMap::new(),
        }
    }

    pub(in crate::llm_pipeline) fn admit(
        &mut self,
        key: &(TraceId, String),
        process: ProcessIdentity,
        stream_key: &str,
    ) -> BindingAdmission {
        if self.positions.contains_key(key) {
            return BindingAdmission::Existing;
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            return BindingAdmission::SequenceExhausted;
        };
        let evicted = (self.counts.get(&key.0).copied().unwrap_or_default()
            >= self.limit_per_trace)
            .then(|| self.oldest_for_trace(key.0))
            .flatten();
        if let Some(evicted) = &evicted {
            self.remove(evicted);
        }
        self.next_sequence = next_sequence;
        let owner = BindingOwner {
            trace_id: key.0,
            process,
            stream_key: stream_key.to_string(),
        };
        self.positions.insert(
            key.clone(),
            BindingPosition {
                sequence: next_sequence,
                owner: owner.clone(),
            },
        );
        self.order.insert((key.0, next_sequence, key.1.clone()));
        *self.counts.entry(key.0).or_default() += 1;
        self.owners.entry(owner).or_default().insert(key.clone());
        evicted.map_or(BindingAdmission::Inserted, BindingAdmission::Evicted)
    }

    pub(in crate::llm_pipeline) fn remove(&mut self, key: &(TraceId, String)) -> bool {
        let Some(position) = self.positions.remove(key) else {
            return false;
        };
        self.order
            .remove(&(key.0, position.sequence, key.1.clone()));
        if let Some(keys) = self.owners.get_mut(&position.owner) {
            keys.remove(key);
            if keys.is_empty() {
                self.owners.remove(&position.owner);
            }
        }
        if let Some(count) = self.counts.get_mut(&key.0) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.counts.remove(&key.0);
            }
        }
        true
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        while let Some(key) = self.oldest_for_trace(trace_id) {
            self.remove(&key);
        }
    }

    pub(in crate::llm_pipeline) fn take_owner(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        stream_key: &str,
    ) -> Vec<(TraceId, String)> {
        let owner = BindingOwner {
            trace_id,
            process,
            stream_key: stream_key.to_string(),
        };
        let keys = self
            .owners
            .get(&owner)
            .map(|keys| keys.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for key in &keys {
            self.remove(key);
        }
        keys
    }

    fn oldest_for_trace(&self, trace_id: TraceId) -> Option<(TraceId, String)> {
        self.order
            .range((Included((trace_id, 0, String::new())), Unbounded))
            .next()
            .filter(|(candidate, _, _)| *candidate == trace_id)
            .map(|(candidate, _, action_id)| (*candidate, action_id.clone()))
    }
}

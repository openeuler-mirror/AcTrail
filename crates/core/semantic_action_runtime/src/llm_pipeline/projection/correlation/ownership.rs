//! Ordered ownership for bounded response-correlation bindings.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use super::LlmStreamKey;

type NodeId = u64;

#[derive(Clone, Eq, Hash, PartialEq)]
struct BindingOwner {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: String,
}

struct OwnershipPosition {
    node_id: NodeId,
    owner: BindingOwner,
}

struct OwnershipNode<K> {
    key: K,
    previous: Option<NodeId>,
    next: Option<NodeId>,
}

#[derive(Default)]
struct TraceOrder {
    oldest: Option<NodeId>,
    newest: Option<NodeId>,
    len: usize,
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
    next_sequence: NodeId,
    positions: HashMap<LlmStreamKey, OwnershipPosition>,
    nodes: HashMap<NodeId, OwnershipNode<LlmStreamKey>>,
    order: HashMap<TraceId, TraceOrder>,
    owners: HashMap<BindingOwner, HashSet<NodeId>>,
}

impl StreamOwnershipIndex {
    pub(in crate::llm_pipeline) fn new(limit_per_trace: usize) -> Self {
        Self {
            limit_per_trace,
            next_sequence: 0,
            positions: HashMap::new(),
            nodes: HashMap::new(),
            order: HashMap::new(),
            owners: HashMap::new(),
        }
    }

    pub(in crate::llm_pipeline) fn admit(&mut self, key: &LlmStreamKey) -> StreamAdmission {
        if self.positions.contains_key(key) {
            return StreamAdmission::Existing;
        }
        let Some(node_id) = self.next_sequence.checked_add(1) else {
            return StreamAdmission::SequenceExhausted;
        };
        let evicted = (self.trace_len(key.trace_id) >= self.limit_per_trace)
            .then(|| self.oldest_for_trace(key.trace_id))
            .flatten();
        if let Some(evicted) = &evicted {
            self.remove(evicted);
        }

        self.next_sequence = node_id;
        let owner = BindingOwner {
            trace_id: key.trace_id,
            process: key.process,
            stream_key: key.stream_key.clone(),
        };
        self.insert_node(node_id, key.clone(), owner);
        evicted.map_or(StreamAdmission::Inserted, StreamAdmission::Evicted)
    }

    pub(in crate::llm_pipeline) fn remove(&mut self, key: &LlmStreamKey) -> bool {
        let Some(position) = self.positions.remove(key) else {
            return false;
        };
        let Some(node) = self.nodes.remove(&position.node_id) else {
            self.remove_owner_node(&position.owner, position.node_id);
            return false;
        };
        self.unlink_node(key.trace_id, position.node_id, node.previous, node.next);
        self.remove_owner_node(&position.owner, position.node_id);
        true
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        while let Some(key) = self.oldest_for_trace(trace_id) {
            self.remove(&key);
        }
    }

    pub(in crate::llm_pipeline) fn keys_for_stream_identity(
        &self,
        trace_id: TraceId,
        process: ProcessIdentity,
        stream_key: &str,
    ) -> Vec<LlmStreamKey> {
        let owner = BindingOwner {
            trace_id,
            process,
            stream_key: stream_key.to_string(),
        };
        self.owners
            .get(&owner)
            .into_iter()
            .flatten()
            .filter_map(|node_id| self.nodes.get(node_id))
            .map(|node| node.key.clone())
            .collect()
    }

    fn insert_node(&mut self, node_id: NodeId, key: LlmStreamKey, owner: BindingOwner) {
        let previous = self.order.get(&key.trace_id).and_then(|order| order.newest);
        if let Some(previous) = previous.and_then(|node_id| self.nodes.get_mut(&node_id)) {
            previous.next = Some(node_id);
        }
        self.nodes.insert(
            node_id,
            OwnershipNode {
                key: key.clone(),
                previous,
                next: None,
            },
        );
        let order = self.order.entry(key.trace_id).or_default();
        order.oldest.get_or_insert(node_id);
        order.newest = Some(node_id);
        order.len += 1;
        self.positions.insert(
            key,
            OwnershipPosition {
                node_id,
                owner: owner.clone(),
            },
        );
        self.owners.entry(owner).or_default().insert(node_id);
    }

    fn unlink_node(
        &mut self,
        trace_id: TraceId,
        node_id: NodeId,
        previous: Option<NodeId>,
        next: Option<NodeId>,
    ) {
        if let Some(previous) = previous.and_then(|node_id| self.nodes.get_mut(&node_id)) {
            previous.next = next;
        }
        if let Some(next) = next.and_then(|node_id| self.nodes.get_mut(&node_id)) {
            next.previous = previous;
        }
        let Some(order) = self.order.get_mut(&trace_id) else {
            return;
        };
        if order.oldest == Some(node_id) {
            order.oldest = next;
        }
        if order.newest == Some(node_id) {
            order.newest = previous;
        }
        order.len = order.len.saturating_sub(1);
        if order.len == 0 {
            self.order.remove(&trace_id);
        }
    }

    fn remove_owner_node(&mut self, owner: &BindingOwner, node_id: NodeId) {
        let Some(nodes) = self.owners.get_mut(owner) else {
            return;
        };
        nodes.remove(&node_id);
        if nodes.is_empty() {
            self.owners.remove(owner);
        }
    }

    fn trace_len(&self, trace_id: TraceId) -> usize {
        self.order.get(&trace_id).map_or(0, |order| order.len)
    }

    fn oldest_for_trace(&self, trace_id: TraceId) -> Option<LlmStreamKey> {
        self.order
            .get(&trace_id)
            .and_then(|order| order.oldest)
            .and_then(|node_id| self.nodes.get(&node_id))
            .map(|node| node.key.clone())
    }
}

/// Keeps per-trace binding state bounded without stale FIFO tokens. The
/// binding payload remains in its owning map; this index owns only admission
/// order and counts.
pub(in crate::llm_pipeline) struct BindingOwnershipIndex {
    limit_per_trace: usize,
    next_sequence: NodeId,
    positions: HashMap<(TraceId, String), OwnershipPosition>,
    nodes: HashMap<NodeId, OwnershipNode<(TraceId, String)>>,
    order: HashMap<TraceId, TraceOrder>,
    owners: HashMap<BindingOwner, HashSet<NodeId>>,
}

impl BindingOwnershipIndex {
    pub(in crate::llm_pipeline) fn new(limit_per_trace: usize) -> Self {
        Self {
            limit_per_trace,
            next_sequence: 0,
            positions: HashMap::new(),
            nodes: HashMap::new(),
            order: HashMap::new(),
            owners: HashMap::new(),
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
        let Some(node_id) = self.next_sequence.checked_add(1) else {
            return BindingAdmission::SequenceExhausted;
        };
        let evicted = (self.trace_len(key.0) >= self.limit_per_trace)
            .then(|| self.oldest_for_trace(key.0))
            .flatten();
        if let Some(evicted) = &evicted {
            self.remove(evicted);
        }

        self.next_sequence = node_id;
        let owner = BindingOwner {
            trace_id: key.0,
            process,
            stream_key: stream_key.to_string(),
        };
        self.insert_node(node_id, key.clone(), owner);
        evicted.map_or(BindingAdmission::Inserted, BindingAdmission::Evicted)
    }

    pub(in crate::llm_pipeline) fn remove(&mut self, key: &(TraceId, String)) -> bool {
        let Some(position) = self.positions.remove(key) else {
            return false;
        };
        let Some(node) = self.nodes.remove(&position.node_id) else {
            self.remove_owner_node(&position.owner, position.node_id);
            return false;
        };
        self.unlink_node(key.0, position.node_id, node.previous, node.next);
        self.remove_owner_node(&position.owner, position.node_id);
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
        let node_ids = self.owners.remove(&owner).unwrap_or_default();
        let keys = node_ids
            .iter()
            .filter_map(|node_id| self.nodes.get(node_id))
            .map(|node| node.key.clone())
            .collect::<Vec<_>>();
        for key in &keys {
            self.remove(key);
        }
        keys
    }

    fn insert_node(&mut self, node_id: NodeId, key: (TraceId, String), owner: BindingOwner) {
        let previous = self.order.get(&key.0).and_then(|order| order.newest);
        if let Some(previous) = previous.and_then(|node_id| self.nodes.get_mut(&node_id)) {
            previous.next = Some(node_id);
        }
        self.nodes.insert(
            node_id,
            OwnershipNode {
                key: key.clone(),
                previous,
                next: None,
            },
        );
        let order = self.order.entry(key.0).or_default();
        order.oldest.get_or_insert(node_id);
        order.newest = Some(node_id);
        order.len += 1;
        self.positions.insert(
            key,
            OwnershipPosition {
                node_id,
                owner: owner.clone(),
            },
        );
        self.owners.entry(owner).or_default().insert(node_id);
    }

    fn unlink_node(
        &mut self,
        trace_id: TraceId,
        node_id: NodeId,
        previous: Option<NodeId>,
        next: Option<NodeId>,
    ) {
        if let Some(previous) = previous.and_then(|node_id| self.nodes.get_mut(&node_id)) {
            previous.next = next;
        }
        if let Some(next) = next.and_then(|node_id| self.nodes.get_mut(&node_id)) {
            next.previous = previous;
        }
        let Some(order) = self.order.get_mut(&trace_id) else {
            return;
        };
        if order.oldest == Some(node_id) {
            order.oldest = next;
        }
        if order.newest == Some(node_id) {
            order.newest = previous;
        }
        order.len = order.len.saturating_sub(1);
        if order.len == 0 {
            self.order.remove(&trace_id);
        }
    }

    fn remove_owner_node(&mut self, owner: &BindingOwner, node_id: NodeId) {
        let Some(nodes) = self.owners.get_mut(owner) else {
            return;
        };
        nodes.remove(&node_id);
        if nodes.is_empty() {
            self.owners.remove(owner);
        }
    }

    fn trace_len(&self, trace_id: TraceId) -> usize {
        self.order.get(&trace_id).map_or(0, |order| order.len)
    }

    fn oldest_for_trace(&self, trace_id: TraceId) -> Option<(TraceId, String)> {
        self.order
            .get(&trace_id)
            .and_then(|order| order.oldest)
            .and_then(|node_id| self.nodes.get(&node_id))
            .map(|node| node.key.clone())
    }
}

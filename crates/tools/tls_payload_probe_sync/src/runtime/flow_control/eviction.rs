//! Eviction policies for the bounded flow-control state map.
//!
//! The controller stores per-stream flow state; the number of distinct streams
//! is unbounded for a long-lived target process, so the map must shed entries.
//! This module isolates *which* entry to shed behind a trait so the policy can
//! be swapped without touching the controller.

use std::collections::HashMap;

use super::types::FlowKey;

/// Decides which stream key to evict when the flow-control state map reaches
/// its configured capacity. Implementations own their bookkeeping.
pub(in crate::runtime) trait EvictionPolicy {
    /// Record an access to `key`, refreshing its recency.
    fn touch(&mut self, key: FlowKey);

    /// Remove and return the next candidate to evict, or `None` when empty.
    fn evict_candidate(&mut self) -> Option<FlowKey>;
}

/// Least-recently-used policy: evicts the stream with the oldest last access.
///
/// `touch` is O(1) — it runs on every payload chunk, so it must be cheap.
/// `evict_candidate` scans in O(n), but it only runs when the map is at
/// capacity and a brand-new stream arrives, which is far less frequent.
#[derive(Debug, Default)]
pub(in crate::runtime) struct LruPolicy {
    /// key -> last-access epoch.
    last_access: HashMap<FlowKey, u64>,
    epoch: u64,
}

impl EvictionPolicy for LruPolicy {
    fn touch(&mut self, key: FlowKey) {
        self.epoch = self.epoch.wrapping_add(1);
        self.last_access.insert(key, self.epoch);
    }

    fn evict_candidate(&mut self) -> Option<FlowKey> {
        let key = self
            .last_access
            .iter()
            .min_by_key(|(_, tick)| **tick)
            .map(|(key, _)| *key)?;
        self.last_access.remove(&key);
        Some(key)
    }
}

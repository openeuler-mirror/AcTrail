//! Fast-path map synchronization from runtime truth to eBPF state.

use std::collections::{BTreeMap, BTreeSet};

use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedProcess {
    pub trace_id: TraceId,
    pub identity: ProcessIdentity,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BindingStateMap {
    by_pid: BTreeMap<u32, TrackedProcess>,
    pids_by_trace: BTreeMap<TraceId, BTreeSet<u32>>,
    capabilities_by_trace: BTreeMap<TraceId, BTreeSet<Capability>>,
}

impl BindingStateMap {
    pub fn track(&mut self, trace_id: TraceId, identity: ProcessIdentity) {
        self.by_pid.insert(
            identity.pid,
            TrackedProcess {
                trace_id,
                identity: identity.clone(),
            },
        );
        self.pids_by_trace
            .entry(trace_id)
            .or_default()
            .insert(identity.pid);
    }

    pub fn track_many(
        &mut self,
        trace_id: TraceId,
        identities: impl IntoIterator<Item = ProcessIdentity>,
    ) {
        for identity in identities {
            self.track(trace_id, identity);
        }
    }

    pub fn set_trace_capabilities(
        &mut self,
        trace_id: TraceId,
        capabilities: impl IntoIterator<Item = Capability>,
    ) {
        self.capabilities_by_trace
            .insert(trace_id, capabilities.into_iter().collect());
    }

    pub fn trace_has_capability(&self, trace_id: TraceId, capability: &Capability) -> bool {
        self.capabilities_by_trace
            .get(&trace_id)
            .map(|capabilities| capabilities.contains(capability))
            .unwrap_or(false)
    }

    pub fn tracked_identity(&self, pid: u32) -> Option<&ProcessIdentity> {
        self.by_pid.get(&pid).map(|entry| &entry.identity)
    }

    pub fn tracked_trace(&self, pid: u32) -> Option<TraceId> {
        self.by_pid.get(&pid).map(|entry| entry.trace_id)
    }

    pub fn remove_pid(&mut self, pid: u32) -> Option<TrackedProcess> {
        let removed = self.by_pid.remove(&pid)?;
        if let Some(pids) = self.pids_by_trace.get_mut(&removed.trace_id) {
            pids.remove(&pid);
            if pids.is_empty() {
                self.pids_by_trace.remove(&removed.trace_id);
            }
        }
        Some(removed)
    }

    pub fn remove_trace(&mut self, trace_id: TraceId) -> Vec<TrackedProcess> {
        self.capabilities_by_trace.remove(&trace_id);
        let Some(pids) = self.pids_by_trace.remove(&trace_id) else {
            return Vec::new();
        };
        pids.into_iter()
            .filter_map(|pid| self.by_pid.remove(&pid))
            .collect()
    }

    pub fn trace_count(&self) -> usize {
        self.pids_by_trace.len()
    }
}

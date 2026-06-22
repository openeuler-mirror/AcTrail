//! Fast-path map synchronization from runtime truth to eBPF state.

use std::collections::{BTreeMap, BTreeSet};

use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::{NamespaceIdentity, ProcessIdentity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedProcess {
    pub trace_id: TraceId,
    pub identity: ProcessIdentity,
    pub map_pid: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BindingStateMap {
    by_pid: BTreeMap<u32, TrackedProcess>,
    by_trace_map_pid: BTreeMap<(TraceId, u32), ProcessIdentity>,
    by_trace_map_process: BTreeMap<(TraceId, u32, u64), ProcessIdentity>,
    pids_by_trace: BTreeMap<TraceId, BTreeSet<u32>>,
    map_pids_by_trace: BTreeMap<TraceId, BTreeSet<u32>>,
    pid_namespace_by_trace: BTreeMap<TraceId, NamespaceIdentity>,
    capabilities_by_trace: BTreeMap<TraceId, BTreeSet<Capability>>,
}

impl BindingStateMap {
    pub fn track(&mut self, trace_id: TraceId, identity: ProcessIdentity) {
        self.track_with_map_pid(trace_id, identity.clone(), identity.pid);
    }

    pub fn track_with_map_pid(
        &mut self,
        trace_id: TraceId,
        identity: ProcessIdentity,
        map_pid: u32,
    ) {
        self.by_pid.insert(
            identity.pid,
            TrackedProcess {
                trace_id,
                identity: identity.clone(),
                map_pid,
            },
        );
        self.by_trace_map_pid
            .insert((trace_id, map_pid), identity.clone());
        self.by_trace_map_process
            .insert((trace_id, map_pid, identity.generation), identity.clone());
        self.pids_by_trace
            .entry(trace_id)
            .or_default()
            .insert(identity.pid);
        self.map_pids_by_trace
            .entry(trace_id)
            .or_default()
            .insert(map_pid);
        if let Some(namespace) = identity.pid_namespace.clone() {
            self.pid_namespace_by_trace
                .entry(trace_id)
                .or_insert(namespace);
        }
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

    pub fn set_trace_pid_namespace(&mut self, trace_id: TraceId, namespace: NamespaceIdentity) {
        self.pid_namespace_by_trace.insert(trace_id, namespace);
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

    pub fn by_host_pid(&self, pid: u32) -> Option<&TrackedProcess> {
        self.by_pid.get(&pid)
    }

    pub fn tracked_event_identity(
        &self,
        trace_id: TraceId,
        map_pid: u32,
        generation: u64,
    ) -> Option<&ProcessIdentity> {
        if generation != 0
            && let Some(identity) = self
                .by_trace_map_process
                .get(&(trace_id, map_pid, generation))
        {
            return Some(identity);
        }
        if generation != 0 {
            return None;
        }
        self.by_trace_map_pid.get(&(trace_id, map_pid))
    }

    pub fn trace_pid_namespace(&self, trace_id: TraceId) -> Option<&NamespaceIdentity> {
        self.pid_namespace_by_trace.get(&trace_id)
    }

    pub fn tracked_trace(&self, pid: u32) -> Option<TraceId> {
        self.by_pid.get(&pid).map(|entry| entry.trace_id)
    }

    pub fn remove_pid(&mut self, pid: u32) -> Option<TrackedProcess> {
        let removed = self.by_pid.remove(&pid)?;
        self.by_trace_map_pid
            .remove(&(removed.trace_id, removed.map_pid));
        if let Some(pids) = self.pids_by_trace.get_mut(&removed.trace_id) {
            pids.remove(&pid);
            if pids.is_empty() {
                self.pids_by_trace.remove(&removed.trace_id);
            }
        }
        if let Some(map_pids) = self.map_pids_by_trace.get_mut(&removed.trace_id) {
            map_pids.remove(&removed.map_pid);
            if map_pids.is_empty() {
                self.map_pids_by_trace.remove(&removed.trace_id);
            }
        }
        Some(removed)
    }

    pub fn remove_event_pid(
        &mut self,
        trace_id: TraceId,
        map_pid: u32,
        generation: u64,
    ) -> Option<TrackedProcess> {
        let identity = self
            .tracked_event_identity(trace_id, map_pid, generation)?
            .clone();
        self.remove_pid(identity.pid)
    }

    pub fn remove_trace(&mut self, trace_id: TraceId) -> Vec<TrackedProcess> {
        self.capabilities_by_trace.remove(&trace_id);
        self.pid_namespace_by_trace.remove(&trace_id);
        if let Some(map_pids) = self.map_pids_by_trace.remove(&trace_id) {
            for map_pid in map_pids {
                self.by_trace_map_pid.remove(&(trace_id, map_pid));
            }
        }
        self.by_trace_map_process
            .retain(|(entry_trace_id, _, _), _| *entry_trace_id != trace_id);
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

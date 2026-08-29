//! Fast-path map synchronization from runtime truth to eBPF state.

use std::collections::{BTreeMap, BTreeSet};

use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::{NamespaceIdentity, ProcessObservation};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedProcess {
    pub trace_id: TraceId,
    pub observation: ProcessObservation,
    pub kernel_tgid: u32,
    pub kernel_start_time: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BindingStateMap {
    by_host_pid: BTreeMap<u32, TrackedProcess>,
    by_trace_kernel_tgid: BTreeMap<(TraceId, u32), TrackedProcess>,
    by_trace_kernel_process: BTreeMap<(TraceId, u32, u64), ProcessObservation>,
    host_pids_by_trace: BTreeMap<TraceId, BTreeSet<u32>>,
    kernel_tgids_by_trace: BTreeMap<TraceId, BTreeSet<u32>>,
    pid_namespace_by_trace: BTreeMap<TraceId, NamespaceIdentity>,
    capabilities_by_trace: BTreeMap<TraceId, BTreeSet<Capability>>,
}

impl BindingStateMap {
    pub fn track_with_kernel_tgid(
        &mut self,
        trace_id: TraceId,
        observation: ProcessObservation,
        kernel_tgid: u32,
        kernel_start_time: u64,
    ) {
        let host_pid = observation.host.as_ref().map(|host| host.pid);
        if let Some(host_pid) = host_pid
            && let Some(previous) = self.by_host_pid.remove(&host_pid)
        {
            self.remove_indexes_for(&previous);
        }
        if let Some(previous) = self.by_trace_kernel_tgid.remove(&(trace_id, kernel_tgid)) {
            if let Some(previous_host_pid) = previous.observation.host.as_ref().map(|host| host.pid)
            {
                self.by_host_pid.remove(&previous_host_pid);
            }
            self.remove_indexes_for(&previous);
        }

        let tracked = TrackedProcess {
            trace_id,
            observation: observation.clone(),
            kernel_tgid,
            kernel_start_time,
        };
        if let Some(host_pid) = host_pid {
            self.by_host_pid.insert(host_pid, tracked.clone());
            self.host_pids_by_trace
                .entry(trace_id)
                .or_default()
                .insert(host_pid);
        }
        self.by_trace_kernel_tgid
            .insert((trace_id, kernel_tgid), tracked);
        self.by_trace_kernel_process.insert(
            (trace_id, kernel_tgid, kernel_start_time),
            observation.clone(),
        );
        self.kernel_tgids_by_trace
            .entry(trace_id)
            .or_default()
            .insert(kernel_tgid);
        if let Some(namespace) = observation.namespace {
            self.pid_namespace_by_trace
                .entry(trace_id)
                .or_insert(namespace.pid_namespace);
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
            .is_some_and(|capabilities| capabilities.contains(capability))
    }

    pub fn by_host_pid(&self, pid: u32) -> Option<&TrackedProcess> {
        self.by_host_pid.get(&pid)
    }

    pub fn tracked_event_observation(
        &self,
        trace_id: TraceId,
        kernel_tgid: u32,
        kernel_start_time: u64,
    ) -> Option<&ProcessObservation> {
        if kernel_start_time != 0 {
            return self
                .by_trace_kernel_process
                .get(&(trace_id, kernel_tgid, kernel_start_time));
        }
        self.by_trace_kernel_tgid
            .get(&(trace_id, kernel_tgid))
            .map(|tracked| &tracked.observation)
    }

    pub fn trace_pid_namespace(&self, trace_id: TraceId) -> Option<&NamespaceIdentity> {
        self.pid_namespace_by_trace.get(&trace_id)
    }

    pub fn remove_pid(&mut self, host_pid: u32) -> Option<TrackedProcess> {
        if let Some(removed) = self.by_host_pid.remove(&host_pid) {
            self.remove_indexes_for(&removed);
            return Some(removed);
        }
        None
    }

    pub fn remove_event_pid(
        &mut self,
        trace_id: TraceId,
        kernel_tgid: u32,
        kernel_start_time: u64,
    ) -> Option<TrackedProcess> {
        if let Some(tracked) = self
            .by_trace_kernel_tgid
            .get(&(trace_id, kernel_tgid))
            .cloned()
        {
            if kernel_start_time != 0 && tracked.kernel_start_time != kernel_start_time {
                return None;
            }
            if let Some(host_pid) = tracked.observation.host.as_ref().map(|host| host.pid) {
                self.by_host_pid.remove(&host_pid);
            }
            self.remove_indexes_for(&tracked);
            return Some(tracked);
        }
        None
    }

    pub fn remove_trace(&mut self, trace_id: TraceId) -> Vec<TrackedProcess> {
        self.capabilities_by_trace.remove(&trace_id);
        self.pid_namespace_by_trace.remove(&trace_id);
        self.by_trace_kernel_process
            .retain(|(entry_trace_id, _, _), _| *entry_trace_id != trace_id);
        let kernel_tgids = self
            .kernel_tgids_by_trace
            .remove(&trace_id)
            .unwrap_or_default();
        let removed = kernel_tgids
            .into_iter()
            .filter_map(|kernel_tgid| self.by_trace_kernel_tgid.remove(&(trace_id, kernel_tgid)))
            .collect::<Vec<_>>();
        for tracked in &removed {
            if let Some(host_pid) = tracked.observation.host.as_ref().map(|host| host.pid) {
                self.by_host_pid.remove(&host_pid);
            }
        }
        self.host_pids_by_trace.remove(&trace_id);
        removed
    }

    pub fn trace_count(&self) -> usize {
        self.kernel_tgids_by_trace.len()
    }

    fn remove_indexes_for(&mut self, tracked: &TrackedProcess) {
        self.by_trace_kernel_tgid
            .remove(&(tracked.trace_id, tracked.kernel_tgid));
        self.by_trace_kernel_process.remove(&(
            tracked.trace_id,
            tracked.kernel_tgid,
            tracked.kernel_start_time,
        ));
        if let Some(host_pid) = tracked.observation.host.as_ref().map(|host| host.pid)
            && let Some(host_pids) = self.host_pids_by_trace.get_mut(&tracked.trace_id)
        {
            host_pids.remove(&host_pid);
            if host_pids.is_empty() {
                self.host_pids_by_trace.remove(&tracked.trace_id);
            }
        }
        if let Some(kernel_tgids) = self.kernel_tgids_by_trace.get_mut(&tracked.trace_id) {
            kernel_tgids.remove(&tracked.kernel_tgid);
            if kernel_tgids.is_empty() {
                self.kernel_tgids_by_trace.remove(&tracked.trace_id);
            }
        }
    }
}

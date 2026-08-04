use super::*;

impl IpcLineageTracker {
    pub(in crate::decode::file_path) fn new(config: IpcLineageConfig, enabled: bool) -> Self {
        assert!(
            config.max_processes_per_trace != 0
                && config.max_candidate_fds_per_trace != 0
                && config.max_stdio_bundles_per_trace != 0,
            "IPC lineage limits must be positive"
        );
        Self {
            enabled,
            config,
            traces: BTreeMap::new(),
            archived_diagnostics: BTreeMap::new(),
            archived_lifecycle: Vec::new(),
            archived_degradations: Vec::new(),
            pending_output_traces: BTreeSet::new(),
        }
    }

    pub(in crate::decode::file_path) fn seed_process(&mut self, key: ProcessFileKey) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let trace = self.traces.entry(key.trace_id).or_default();
        if let Err(reason) = trace.observe_process(&key.process, &self.config) {
            trace.disable(key.trace_id, key.process, reason, 0);
        }
    }

    pub(in crate::decode::file_path) fn inherit_process(
        &mut self,
        parent: &ProcessFileKey,
        child: ProcessFileKey,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(child.trace_id);
        let trace = self.traces.entry(child.trace_id).or_default();
        if trace.disabled_reason.is_some() {
            return;
        }
        let Some(parent_id) = LineageProcessId::from_observation(&parent.process) else {
            trace.increment_diagnostic("fork_parent_identity_missing");
            return;
        };
        let inherited = trace
            .processes
            .get(&parent_id)
            .map(|state| {
                state
                    .fds
                    .iter()
                    .map(|(fd, binding)| (*fd, binding.inherited()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !trace.processes.contains_key(&parent_id) {
            trace.increment_diagnostic("fork_parent_lineage_missing");
        }
        let child_id = match trace.observe_process(&child.process, &self.config) {
            Ok(child_id) => child_id,
            Err(reason) => {
                trace.disable(child.trace_id, child.process, reason, 0);
                return;
            }
        };
        trace.parents.insert(child_id, parent_id);
        for (fd, binding) in inherited {
            if let Err(reason) = trace.bind_fd(child_id, fd, binding, &self.config) {
                trace.disable(child.trace_id, child.process.clone(), reason, 0);
                return;
            }
        }
    }

    pub(in crate::decode::file_path) fn rekey_process(
        &mut self,
        source: &ProcessFileKey,
        target: ProcessFileKey,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(target.trace_id);
        let trace = self.traces.entry(target.trace_id).or_default();
        let source_id = LineageProcessId::from_observation(&source.process);
        let target_id = LineageProcessId::from_observation(&target.process);
        if source_id != target_id {
            trace.increment_diagnostic("process_identity_rekey_mismatch");
            return;
        }
        if let Err(reason) = trace.observe_process(&target.process, &self.config) {
            trace.disable(target.trace_id, target.process, reason, 0);
        }
    }

    pub(in crate::decode::file_path) fn exec_process(
        &mut self,
        key: ProcessFileKey,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let trace = self.traces.entry(key.trace_id).or_default();
        let process = match trace.observe_process(&key.process, &self.config) {
            Ok(process) => process,
            Err(reason) => {
                trace.disable(key.trace_id, key.process, reason, observed_ktime_ns);
                return;
            }
        };
        let close_on_exec_fds = trace
            .processes
            .get(&process)
            .map(|state| {
                state
                    .fds
                    .iter()
                    .filter_map(|(fd, binding)| binding.close_on_exec.then_some(*fd))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut changed = BTreeSet::new();
        for fd in close_on_exec_fds {
            if let Some(binding) = trace.unbind_fd(process, fd) {
                changed.insert(binding.channel_id);
            }
        }
        if let Some(state) = trace.processes.get_mut(&process) {
            state.exec_ktime_ns = Some(observed_ktime_ns);
        }
        let servers = trace.dependent_servers(process, &changed);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "exec",
            &self.config,
        );
    }

    pub(in crate::decode::file_path) fn exit_process(
        &mut self,
        key: &ProcessFileKey,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let Some(process) = LineageProcessId::from_observation(&key.process) else {
            return;
        };
        let Some(trace) = self.traces.get_mut(&key.trace_id) else {
            return;
        };
        trace.close_bundle(key.trace_id, process, observed_ktime_ns, "process_exit");
        let channels = trace
            .processes
            .get(&process)
            .map(|state| {
                state
                    .fds
                    .values()
                    .map(|binding| binding.channel_id.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let mut servers = trace.dependent_servers(process, &channels);
        servers.remove(&process);
        trace.remove_process_state(process);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "peer_process_exit",
            &self.config,
        );
    }

    pub(in crate::decode::file_path) fn remove_trace(&mut self, trace_id: TraceId) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.remove(&trace_id);
        let Some(mut trace) = self.traces.remove(&trace_id) else {
            return;
        };
        for (reason, count) in trace.diagnostics {
            let archived = self.archived_diagnostics.entry(reason).or_default();
            *archived = archived.saturating_add(count);
        }
        self.archived_lifecycle.append(&mut trace.pending_lifecycle);
        self.archived_degradations
            .append(&mut trace.pending_degradations);
    }

    pub(in crate::decode::file_path) fn resolve_kind(
        &self,
        key: &ProcessFileKey,
        fd: u32,
    ) -> Option<FdIpcKind> {
        if !self.enabled {
            return None;
        }
        let process = LineageProcessId::from_observation(&key.process)?;
        self.traces
            .get(&key.trace_id)?
            .processes
            .get(&process)?
            .fds
            .get(&fd)
            .map(|binding| binding.kind)
    }

    pub(in crate::decode::file_path) fn record_pair(
        &mut self,
        key: &ProcessFileKey,
        kind: FdIpcKind,
        fd_a: u32,
        fd_b: u32,
        close_on_exec: bool,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let trace = self.traces.entry(key.trace_id).or_default();
        let process = match trace.observe_process(&key.process, &self.config) {
            Ok(process) => process,
            Err(reason) => {
                trace.disable(key.trace_id, key.process.clone(), reason, observed_ktime_ns);
                return;
            }
        };
        let channel_id = IpcChannelId {
            creator_process: process,
            created_ktime_ns: observed_ktime_ns,
            fd_a,
            fd_b,
        };
        let (side_a, side_b) = match kind {
            FdIpcKind::Pipe => (IpcEndpointSide::Read, IpcEndpointSide::Write),
            FdIpcKind::UnixSocket => (IpcEndpointSide::A, IpcEndpointSide::B),
        };
        let mut changed = match trace.bind_fd(
            process,
            fd_a,
            IpcEndpointBinding::created(channel_id.clone(), kind, side_a, close_on_exec),
            &self.config,
        ) {
            Ok(changed) => changed,
            Err(reason) => {
                trace.disable(key.trace_id, key.process.clone(), reason, observed_ktime_ns);
                return;
            }
        };
        match trace.bind_fd(
            process,
            fd_b,
            IpcEndpointBinding::created(channel_id, kind, side_b, close_on_exec),
            &self.config,
        ) {
            Ok(other) => changed.extend(other),
            Err(reason) => {
                trace.disable(key.trace_id, key.process.clone(), reason, observed_ktime_ns);
                return;
            }
        }
        let servers = trace.dependent_servers(process, &changed);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "ipc_created",
            &self.config,
        );
    }

    pub(in crate::decode::file_path) fn replace_with_non_ipc(
        &mut self,
        key: &ProcessFileKey,
        fd: u32,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let config = self.config;
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        let Some(binding) = trace.unbind_fd(process, fd) else {
            return;
        };
        let channels = BTreeSet::from([binding.channel_id]);
        let servers = trace.dependent_servers(process, &channels);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "fd_rebound",
            &config,
        );
    }

    pub(in crate::decode::file_path) fn close_fd(
        &mut self,
        key: &ProcessFileKey,
        fd: u32,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let config = self.config;
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        let Some(binding) = trace.unbind_fd(process, fd) else {
            return;
        };
        let channels = BTreeSet::from([binding.channel_id]);
        let servers = trace.dependent_servers(process, &channels);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "fd_closed",
            &config,
        );
    }

    pub(in crate::decode::file_path) fn close_range(
        &mut self,
        key: &ProcessFileKey,
        first: u32,
        last: u32,
        close_on_exec: bool,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let config = self.config;
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        let fds = trace
            .processes
            .get(&process)
            .map(|state| {
                state
                    .fds
                    .range(first..=last)
                    .map(|(fd, _)| *fd)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if close_on_exec {
            if let Some(state) = trace.processes.get_mut(&process) {
                for fd in fds {
                    if let Some(binding) = state.fds.get_mut(&fd) {
                        binding.close_on_exec = true;
                    }
                }
            }
            return;
        }
        let mut changed = BTreeSet::new();
        for fd in fds {
            if let Some(binding) = trace.unbind_fd(process, fd) {
                changed.insert(binding.channel_id);
            }
        }
        if changed.is_empty() {
            return;
        }
        let servers = trace.dependent_servers(process, &changed);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "fd_range_closed",
            &config,
        );
    }

    pub(in crate::decode::file_path) fn set_fd_close_on_exec(
        &mut self,
        key: &ProcessFileKey,
        fd: u32,
        close_on_exec: bool,
    ) {
        if !self.enabled {
            return;
        }
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        if let Some(binding) = trace
            .processes
            .get_mut(&process)
            .and_then(|state| state.fds.get_mut(&fd))
        {
            binding.close_on_exec = close_on_exec;
        }
    }

    pub(in crate::decode::file_path) fn duplicate_fd(
        &mut self,
        key: &ProcessFileKey,
        source_fd: u32,
        target_fd: u32,
        close_on_exec: bool,
        observed_ktime_ns: u64,
    ) {
        if !self.enabled {
            return;
        }
        if source_fd == target_fd {
            return;
        }
        self.pending_output_traces.insert(key.trace_id);
        let config = self.config;
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        let source = trace
            .processes
            .get(&process)
            .and_then(|state| state.fds.get(&source_fd))
            .cloned();
        let changed = match source {
            Some(binding) => match trace.bind_fd(
                process,
                target_fd,
                binding.duplicated(close_on_exec),
                &config,
            ) {
                Ok(changed) => changed,
                Err(reason) => {
                    trace.disable(key.trace_id, key.process.clone(), reason, observed_ktime_ns);
                    return;
                }
            },
            None => {
                let Some(binding) = trace.unbind_fd(process, target_fd) else {
                    return;
                };
                BTreeSet::from([binding.channel_id])
            }
        };
        let servers = trace.dependent_servers(process, &changed);
        trace.refresh_servers(
            key.trace_id,
            servers,
            observed_ktime_ns,
            "fd_duplicated",
            &config,
        );
    }

    pub(in crate::decode::file_path) fn take_lifecycle_events(&mut self) -> Vec<RawCollectorEvent> {
        if !self.enabled {
            return Vec::new();
        }
        let mut lifecycle = std::mem::take(&mut self.archived_lifecycle);
        let mut degradations = std::mem::take(&mut self.archived_degradations);
        for trace_id in std::mem::take(&mut self.pending_output_traces) {
            let Some(trace) = self.traces.get_mut(&trace_id) else {
                continue;
            };
            for (reason, count) in std::mem::take(&mut trace.diagnostics) {
                let archived = self.archived_diagnostics.entry(reason).or_default();
                *archived = archived.saturating_add(count);
            }
            lifecycle.append(&mut trace.pending_lifecycle);
            degradations.append(&mut trace.pending_degradations);
        }
        let mut events = lifecycle
            .into_iter()
            .map(StdioBundleLifecycle::into_raw_event)
            .collect::<Vec<_>>();
        events.extend(
            degradations
                .into_iter()
                .map(StdioLineageDiagnostic::into_raw_event),
        );
        events
    }

    pub(in crate::decode::file_path) fn lineage_gap_diagnostics(&self) -> Vec<(&'static str, u64)> {
        if !self.enabled {
            return Vec::new();
        }
        let mut diagnostics = self.archived_diagnostics.clone();
        for trace_id in &self.pending_output_traces {
            let Some(trace) = self.traces.get(trace_id) else {
                continue;
            };
            for (reason, count) in &trace.diagnostics {
                let total = diagnostics.entry(reason).or_default();
                *total = total.saturating_add(*count);
            }
        }
        diagnostics.into_iter().collect()
    }

    fn trace_process_mut(
        &mut self,
        key: &ProcessFileKey,
    ) -> Option<(&mut TraceLineageState, LineageProcessId)> {
        let process = LineageProcessId::from_observation(&key.process)?;
        let trace = self.traces.get_mut(&key.trace_id)?;
        (trace.processes.contains_key(&process) && trace.disabled_reason.is_none())
            .then_some((trace, process))
    }
}

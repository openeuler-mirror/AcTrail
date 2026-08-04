use super::*;

impl IpcLineageTracker {
    pub(in crate::decode::file_path) fn new(
        config: IpcLineageConfig,
        mcp_stdio_projection_enabled: bool,
    ) -> Self {
        assert!(
            config.max_processes_per_trace != 0
                && config.max_candidate_fds_per_trace != 0
                && config.max_stdio_bundles_per_trace != 0,
            "IPC lineage limits must be positive"
        );
        Self {
            collection_enabled: config.enabled,
            mcp_stdio_projection_enabled,
            config,
            traces: BTreeMap::new(),
            archived_diagnostics: BTreeMap::new(),
            archived_lifecycle: Vec::new(),
            archived_degradations: Vec::new(),
            pending_output_traces: BTreeSet::new(),
        }
    }

    pub(in crate::decode::file_path) fn resolve_kind(
        &self,
        key: &ProcessFileKey,
        fd: u32,
    ) -> Option<FdIpcKind> {
        if !self.collection_enabled {
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
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
        let trace = self.traces.entry(key.trace_id).or_default();
        let process = match trace.observe_process(&key.process, &self.config) {
            Ok(process) => process,
            Err(reason) => {
                trace.disable(
                    key.trace_id,
                    key.process.clone(),
                    reason,
                    observed_ktime_ns,
                    projection_enabled,
                );
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
                trace.disable(
                    key.trace_id,
                    key.process.clone(),
                    reason,
                    observed_ktime_ns,
                    projection_enabled,
                );
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
                trace.disable(
                    key.trace_id,
                    key.process.clone(),
                    reason,
                    observed_ktime_ns,
                    projection_enabled,
                );
                return;
            }
        }
        if projection_enabled {
            let servers = trace.dependent_servers(process, &changed);
            trace.refresh_servers(
                key.trace_id,
                servers,
                observed_ktime_ns,
                "ipc_created",
                &self.config,
            );
        }
    }

    pub(in crate::decode::file_path) fn replace_with_non_ipc(
        &mut self,
        key: &ProcessFileKey,
        fd: u32,
        observed_ktime_ns: u64,
    ) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
        let config = self.config;
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        let Some(binding) = trace.unbind_fd(process, fd) else {
            return;
        };
        if projection_enabled {
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
    }

    pub(in crate::decode::file_path) fn close_fd(
        &mut self,
        key: &ProcessFileKey,
        fd: u32,
        observed_ktime_ns: u64,
    ) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
        let config = self.config;
        let Some((trace, process)) = self.trace_process_mut(key) else {
            return;
        };
        let Some(binding) = trace.unbind_fd(process, fd) else {
            return;
        };
        if projection_enabled {
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
    }

    pub(in crate::decode::file_path) fn close_range(
        &mut self,
        key: &ProcessFileKey,
        first: u32,
        last: u32,
        close_on_exec: bool,
        observed_ktime_ns: u64,
    ) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
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
        if projection_enabled {
            let servers = trace.dependent_servers(process, &changed);
            trace.refresh_servers(
                key.trace_id,
                servers,
                observed_ktime_ns,
                "fd_range_closed",
                &config,
            );
        }
    }

    pub(in crate::decode::file_path) fn set_fd_close_on_exec(
        &mut self,
        key: &ProcessFileKey,
        fd: u32,
        close_on_exec: bool,
    ) {
        if !self.collection_enabled {
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
        if !self.collection_enabled {
            return;
        }
        if source_fd == target_fd {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
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
                    trace.disable(
                        key.trace_id,
                        key.process.clone(),
                        reason,
                        observed_ktime_ns,
                        projection_enabled,
                    );
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
        if projection_enabled {
            let servers = trace.dependent_servers(process, &changed);
            trace.refresh_servers(
                key.trace_id,
                servers,
                observed_ktime_ns,
                "fd_duplicated",
                &config,
            );
        }
    }

    pub(in crate::decode::file_path) fn take_lifecycle_events(&mut self) -> Vec<RawCollectorEvent> {
        if !self.mcp_stdio_projection_enabled {
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
        if !self.mcp_stdio_projection_enabled {
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
        if !self.collection_enabled {
            return None;
        }
        let process = LineageProcessId::from_observation(&key.process)?;
        let trace = self.traces.get_mut(&key.trace_id)?;
        (trace.processes.contains_key(&process) && trace.disabled_reason.is_none())
            .then_some((trace, process))
    }
}

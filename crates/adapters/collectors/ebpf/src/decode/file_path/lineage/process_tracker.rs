use super::*;

impl IpcLineageTracker {
    pub(in crate::decode::file_path) fn seed_process(&mut self, key: ProcessFileKey) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
        let trace = self.traces.entry(key.trace_id).or_default();
        if let Err(reason) = trace.observe_process(&key.process, &self.config) {
            trace.disable(key.trace_id, key.process, reason, 0, projection_enabled);
        }
    }

    pub(in crate::decode::file_path) fn inherit_process(
        &mut self,
        parent: &ProcessFileKey,
        child: ProcessFileKey,
    ) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(child.trace_id);
        }
        let trace = self.traces.entry(child.trace_id).or_default();
        if trace.disabled_reason.is_some() {
            return;
        }
        let Some(parent_id) = LineageProcessId::from_observation(&parent.process) else {
            if projection_enabled {
                trace.increment_diagnostic("fork_parent_identity_missing");
            }
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
        if projection_enabled && !trace.processes.contains_key(&parent_id) {
            trace.increment_diagnostic("fork_parent_lineage_missing");
        }
        let child_id = match trace.observe_process(&child.process, &self.config) {
            Ok(child_id) => child_id,
            Err(reason) => {
                trace.disable(child.trace_id, child.process, reason, 0, projection_enabled);
                return;
            }
        };
        if projection_enabled {
            trace.parents.insert(child_id, parent_id);
        }
        for (fd, binding) in inherited {
            if let Err(reason) = trace.bind_fd(child_id, fd, binding, &self.config) {
                trace.disable(
                    child.trace_id,
                    child.process.clone(),
                    reason,
                    0,
                    projection_enabled,
                );
                return;
            }
        }
    }

    pub(in crate::decode::file_path) fn rekey_process(
        &mut self,
        source: &ProcessFileKey,
        target: ProcessFileKey,
    ) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(target.trace_id);
        }
        let trace = self.traces.entry(target.trace_id).or_default();
        let source_id = LineageProcessId::from_observation(&source.process);
        let target_id = LineageProcessId::from_observation(&target.process);
        if source_id != target_id {
            if projection_enabled {
                trace.increment_diagnostic("process_identity_rekey_mismatch");
            }
            return;
        }
        if let Err(reason) = trace.observe_process(&target.process, &self.config) {
            trace.disable(
                target.trace_id,
                target.process,
                reason,
                0,
                projection_enabled,
            );
        }
    }

    pub(in crate::decode::file_path) fn exec_process(
        &mut self,
        key: ProcessFileKey,
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
                    key.process,
                    reason,
                    observed_ktime_ns,
                    projection_enabled,
                );
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
        if projection_enabled {
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
    }

    pub(in crate::decode::file_path) fn exit_process(
        &mut self,
        key: &ProcessFileKey,
        observed_ktime_ns: u64,
    ) {
        if !self.collection_enabled {
            return;
        }
        let projection_enabled = self.mcp_stdio_projection_enabled;
        if projection_enabled {
            self.pending_output_traces.insert(key.trace_id);
        }
        let Some(process) = LineageProcessId::from_observation(&key.process) else {
            return;
        };
        let Some(trace) = self.traces.get_mut(&key.trace_id) else {
            return;
        };
        let servers = projection_enabled.then(|| {
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
            servers
        });
        trace.remove_process_state(process);
        if let Some(servers) = servers {
            trace.refresh_servers(
                key.trace_id,
                servers,
                observed_ktime_ns,
                "peer_process_exit",
                &self.config,
            );
        }
    }

    pub(in crate::decode::file_path) fn remove_trace(&mut self, trace_id: TraceId) {
        if !self.collection_enabled {
            return;
        }
        self.pending_output_traces.remove(&trace_id);
        let Some(mut trace) = self.traces.remove(&trace_id) else {
            return;
        };
        if !self.mcp_stdio_projection_enabled {
            return;
        }
        for (reason, count) in trace.diagnostics {
            let archived = self.archived_diagnostics.entry(reason).or_default();
            *archived = archived.saturating_add(count);
        }
        self.archived_lifecycle.append(&mut trace.pending_lifecycle);
        self.archived_degradations
            .append(&mut trace.pending_degradations);
    }
}

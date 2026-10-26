use std::collections::BTreeSet;

use config_core::daemon::IpcLineageConfig;
use model_core::ids::TraceId;

use super::bundle::{StdioBundle, StdioBundleId, StdioBundleLifecycle};
use super::{IpcChannelId, IpcEndpointBinding, LineageProcessId, TraceLineageState};

impl TraceLineageState {
    pub(super) fn dependent_servers(
        &self,
        process: LineageProcessId,
        channels: &BTreeSet<IpcChannelId>,
    ) -> BTreeSet<LineageProcessId> {
        let mut servers = BTreeSet::from([process]);
        for channel in channels {
            if let Some(dependents) = self.bundles_by_channel.get(channel) {
                servers.extend(dependents.iter().copied());
            }
        }
        servers
    }

    pub(super) fn refresh_servers(
        &mut self,
        trace_id: TraceId,
        servers: BTreeSet<LineageProcessId>,
        observed_ktime_ns: u64,
        cause: &'static str,
        config: &IpcLineageConfig,
    ) {
        for server in servers {
            if self.processes.contains_key(&server) {
                self.refresh_process_bundle(trace_id, server, observed_ktime_ns, cause, config);
            }
        }
    }

    fn refresh_process_bundle(
        &mut self,
        trace_id: TraceId,
        process: LineageProcessId,
        observed_ktime_ns: u64,
        cause: &'static str,
        config: &IpcLineageConfig,
    ) {
        match self.build_bundle(process) {
            Ok(Some(bundle)) => {
                self.last_gaps.remove(&process);
                match self.active_bundles.get(&process) {
                    Some(active) if active == &bundle => {}
                    Some(_) => {
                        self.unregister_bundle(process);
                        self.active_bundles.insert(process, bundle.clone());
                        self.register_bundle(process, &bundle);
                        self.queue_lifecycle(
                            trace_id,
                            process,
                            "replaced",
                            bundle,
                            observed_ktime_ns,
                            Some(cause),
                        );
                    }
                    None => {
                        if self.active_bundles.len() >= config.max_stdio_bundles_per_trace as usize
                        {
                            if self.record_gap(process, "stdio_bundle_capacity_exhausted") {
                                self.queue_lineage_capacity_diagnostic(
                                    trace_id,
                                    process,
                                    observed_ktime_ns,
                                    "stdio_bundle_capacity_exhausted",
                                );
                            }
                            return;
                        }
                        self.active_bundles.insert(process, bundle.clone());
                        self.register_bundle(process, &bundle);
                        self.queue_lifecycle(
                            trace_id,
                            process,
                            "ready",
                            bundle,
                            observed_ktime_ns,
                            None,
                        );
                    }
                }
            }
            Ok(None) => {
                if self.active_bundles.contains_key(&process) {
                    let _ = self.record_gap(process, cause);
                } else {
                    self.last_gaps.remove(&process);
                }
                self.close_bundle(trace_id, process, observed_ktime_ns, cause);
            }
            Err(reason) => {
                let _ = self.record_gap(process, reason);
                self.close_bundle(trace_id, process, observed_ktime_ns, reason);
            }
        }
    }

    fn register_bundle(&mut self, process: LineageProcessId, bundle: &StdioBundle) {
        for channel in bundle.channels() {
            self.bundles_by_channel
                .entry(channel)
                .or_default()
                .insert(process);
        }
    }

    fn unregister_bundle(&mut self, process: LineageProcessId) {
        let Some(bundle) = self.active_bundles.get(&process) else {
            return;
        };
        for channel in bundle.channels() {
            let remove_channel = if let Some(servers) = self.bundles_by_channel.get_mut(&channel) {
                servers.remove(&process);
                servers.is_empty()
            } else {
                false
            };
            if remove_channel {
                self.bundles_by_channel.remove(&channel);
            }
        }
    }

    pub(super) fn close_bundle(
        &mut self,
        trace_id: TraceId,
        process: LineageProcessId,
        observed_ktime_ns: u64,
        reason: &'static str,
    ) {
        self.unregister_bundle(process);
        let Some(bundle) = self.active_bundles.remove(&process) else {
            return;
        };
        self.queue_lifecycle(
            trace_id,
            process,
            "closed",
            bundle,
            observed_ktime_ns,
            Some(reason),
        );
    }

    fn queue_lifecycle(
        &mut self,
        trace_id: TraceId,
        process: LineageProcessId,
        operation: &'static str,
        bundle: StdioBundle,
        observed_ktime_ns: u64,
        reason: Option<&'static str>,
    ) {
        let Some(server) = self
            .processes
            .get(&process)
            .map(|state| state.observation.clone())
        else {
            self.increment_diagnostic("stdio_bundle_server_observation_missing");
            return;
        };
        self.pending_lifecycle.push(StdioBundleLifecycle {
            trace_id,
            server,
            operation,
            bundle,
            observed_ktime_ns,
            reason,
        });
    }

    fn queue_lineage_capacity_diagnostic(
        &mut self,
        trace_id: TraceId,
        process: LineageProcessId,
        observed_ktime_ns: u64,
        reason: &'static str,
    ) {
        let Some(observation) = self
            .processes
            .get(&process)
            .map(|state| state.observation.clone())
        else {
            self.increment_diagnostic("stdio_bundle_server_observation_missing");
            return;
        };
        self.pending_degradations
            .push(super::bundle::StdioLineageDiagnostic {
                trace_id,
                process: observation,
                operation: "lineage_capacity_exhausted",
                observed_ktime_ns,
                reason,
            });
    }

    fn build_bundle(&self, process: LineageProcessId) -> Result<Option<StdioBundle>, &'static str> {
        let Some(state) = self.processes.get(&process) else {
            return Ok(None);
        };
        let Some(exec_ktime_ns) = state.exec_ktime_ns else {
            return Ok(None);
        };
        let stdin = state.fds.get(&0).cloned();
        let stdout = state.fds.get(&1).cloned();
        if stdin.is_none() && stdout.is_none() {
            return Ok(None);
        }
        let stdin = stdin.ok_or("missing_stdin_lineage")?;
        let stdout = stdout.ok_or("missing_stdout_lineage")?;
        if !stdin.supports_server_stdin() {
            return Err("invalid_stdin_direction");
        }
        if !stdout.supports_server_output() {
            return Err("invalid_stdout_direction");
        }
        let ancestors = self.ancestor_chain(process);
        let stdin_peers = self.ancestor_peer_owners(&stdin, &ancestors);
        if stdin_peers.is_empty() {
            return Err("missing_stdin_ancestor_peer");
        }
        let stdout_peers = self.ancestor_peer_owners(&stdout, &ancestors);
        if stdout_peers.is_empty() {
            return Err("missing_stdout_ancestor_peer");
        }
        let client = ancestors
            .iter()
            .find(|ancestor| stdin_peers.contains(*ancestor) && stdout_peers.contains(*ancestor))
            .copied()
            .ok_or("stdio_peer_client_mismatch")?;
        let stderr = state.fds.get(&2).filter(|binding| {
            binding.supports_server_output()
                && self
                    .ancestor_peer_owners(binding, &ancestors)
                    .contains(&client)
        });
        Ok(Some(StdioBundle {
            id: StdioBundleId {
                server: process,
                exec_ktime_ns,
            },
            stdin,
            stdout,
            stderr: stderr.cloned(),
            client,
        }))
    }

    fn ancestor_chain(&self, process: LineageProcessId) -> Vec<LineageProcessId> {
        let mut ancestors = Vec::new();
        let mut cursor = process;
        let mut visited = BTreeSet::new();
        while let Some(parent) = self.parents.get(&cursor).copied() {
            if !visited.insert(parent) {
                break;
            }
            ancestors.push(parent);
            cursor = parent;
        }
        ancestors
    }

    fn ancestor_peer_owners(
        &self,
        binding: &IpcEndpointBinding,
        ancestors: &[LineageProcessId],
    ) -> BTreeSet<LineageProcessId> {
        let ancestor_set = ancestors.iter().copied().collect::<BTreeSet<_>>();
        self.owners_by_channel
            .get(&binding.channel_id)
            .into_iter()
            .flatten()
            .filter(|owner| owner.side == binding.side.opposite())
            .filter(|owner| ancestor_set.contains(&owner.process))
            .map(|owner| owner.process)
            .collect()
    }
}

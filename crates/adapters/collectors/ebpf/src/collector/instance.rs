//! Collector contract implementation for the eBPF adapter.

use super::collector_trace_binding::{cleanup_suppressed_fds_for_trace, kernel_start_time};
use super::*;

impl CollectorInstance for EbpfCollector {
    fn descriptor(&self) -> &collector_capability::CollectorDescriptor {
        &self.probe_result.descriptor
    }

    fn install_coverage_guard(
        &mut self,
        _request: &CoverageGuardRequest,
    ) -> Result<CoverageGuardHandle, CollectorError> {
        Err(CollectorError::new(
            "coverage_guard",
            "current libbpf-rs collector path does not implement attach coverage guard",
        ))
    }

    fn bind_trace(
        &mut self,
        request: &TraceBindingRequest,
    ) -> Result<TraceBindingHandle, CollectorError> {
        if let Some(reason) = &self.probe_result.reason_unavailable {
            return Err(CollectorError::new("bind_trace", reason.clone()));
        }
        if let Some(unsupported_required) = request.requested_capabilities.iter().find(|request| {
            !supported_required_capability(
                &request.capability,
                self.loader.config(),
                self.loader.payload_config(),
            ) && request.mode == RequestMode::Required
        }) {
            return Err(CollectorError::new(
                "bind_trace",
                format!(
                    "current libbpf-rs collector path does not support required capability {:?}",
                    unsupported_required.capability
                ),
            ));
        }

        self.ensure_runtime_for_requests(&request.requested_capabilities)?;
        let root_start_time = kernel_start_time(&request.root_observation)?;
        let root_map_pid = self.map_pid_for_observation(&request.root_observation)?;
        let root_pid_namespace = request
            .root_observation
            .namespace
            .as_ref()
            .map(|value| value.pid_namespace.clone())
            .ok_or_else(|| {
                CollectorError::new("pid_namespace", "root process has no PID namespace")
            })?;
        let attached_capabilities = self.runtime_ref()?.attached_capabilities().clone();
        self.register_trace_pid_namespace(request.trace_id, &request.root_observation)?;
        let runtime = self.runtime_mut()?;
        if let Err(error) = runtime.track_pid(root_map_pid, root_start_time, request.trace_id) {
            let _ = runtime.unregister_trace_pid_namespace(request.trace_id);
            return Err(loader_error(error));
        }
        if let Err(error) = self.register_initial_suppressed_fds(
            request.trace_id,
            root_start_time,
            root_map_pid,
            &request.initial_suppressed_fds,
        ) {
            let _ = self
                .runtime_mut()
                .and_then(|runtime| runtime.untrack_pid(root_map_pid).map_err(loader_error));
            let _ = self.runtime_mut().and_then(|runtime| {
                runtime
                    .unregister_trace_pid_namespace(request.trace_id)
                    .map_err(loader_error)
            });
            return Err(error);
        }
        self.bindings.set_trace_capabilities(
            request.trace_id,
            request
                .requested_capabilities
                .iter()
                .filter(|request| request.mode != RequestMode::Disabled)
                .filter(|request| attached_capabilities.contains(&request.capability))
                .map(|request| request.capability.clone()),
        );
        self.bindings
            .set_trace_pid_namespace(request.trace_id, root_pid_namespace);
        self.bindings.track_with_map_pid(
            request.trace_id,
            request.root_observation.clone(),
            root_map_pid,
            root_start_time,
        );
        self.file_tracker.seed_process(
            request.trace_id,
            request.root_observation.clone(),
            request
                .root_observation
                .host
                .as_ref()
                .and_then(|host| crate::procfs::read_process_cwd(host.pid)),
        );
        Ok(TraceBindingHandle {
            collector: self.probe_result.descriptor.clone(),
            bound_at: SystemTime::now(),
        })
    }

    fn unbind_trace(&mut self, trace_id: TraceId) -> Result<(), CollectorError> {
        // Flush any aggregated-but-unflushed net events for this trace into the
        // backlog so they are emitted on the next poll instead of being dropped.
        self.net_aggregation_backlog
            .extend(self.net_aggregator.flush_trace(trace_id));
        self.stdio_payloads.release_trace(trace_id);
        self.cancel_pending_launch(trace_id)?;
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.untrack_fork_trace(trace_id).map_err(loader_error)?;
            cleanup_suppressed_fds_for_trace(runtime, &mut self.suppressed_fds, trace_id)?;
            for tracked in self.bindings.remove_trace(trace_id) {
                runtime.untrack_pid(tracked.map_pid).map_err(loader_error)?;
            }
            runtime
                .unregister_trace_pid_namespace(trace_id)
                .map_err(loader_error)?;
        } else {
            let _ = self.bindings.remove_trace(trace_id);
        }
        self.file_tracker.remove_trace(trace_id);
        Ok(())
    }

    fn poll_batch(&mut self) -> Result<CollectorPollBatch, CollectorError> {
        self.poll_batch_impl()
    }

    fn stats(&self) -> CollectorStats {
        let mut dropped = Vec::new();
        if self.binding_gap_drops != 0 {
            dropped.push(DropCounter {
                reason: "ebpf_file_identity_binding_gap".to_string(),
                count: self.binding_gap_drops,
            });
        }
        if self.binding_gap_lifecycle_skips != 0 {
            dropped.push(DropCounter {
                reason: "ebpf_exit_lifecycle_binding_gap".to_string(),
                count: self.binding_gap_lifecycle_skips,
            });
        }
        for (reason, count) in self.file_tracker.lineage_gap_diagnostics() {
            dropped.push(DropCounter {
                reason: format!("ebpf_stdio_bundle_lineage_gap:{reason}"),
                count,
            });
        }
        self.stdio_payloads.append_drop_counters(&mut dropped);
        CollectorStats {
            collector_name: CollectorName::new("ebpf"),
            active_bindings: self.active_binding_trace_count(),
            last_heartbeat_at: SystemTime::now(),
            dropped,
        }
    }

    fn active_binding_trace_count(&self) -> usize {
        self.active_binding_trace_count()
    }
}

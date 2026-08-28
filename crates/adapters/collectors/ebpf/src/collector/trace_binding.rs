//! Trace-to-kernel binding lifecycle for the eBPF collector.

use super::*;

impl EbpfCollector {
    pub fn fork_trace_lookup(&self, host_pid: u32) -> Result<ForkTraceLookup, CollectorError> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(ForkTraceLookup::Unavailable);
        };
        if !runtime.is_attached() {
            return Ok(ForkTraceLookup::Unavailable);
        }
        let binding = runtime.fork_trace_binding(host_pid).map_err(loader_error)?;
        let Some((kernel_tgid, binding)) = binding else {
            let failed_publications = runtime
                .fork_identity_publish_failures()
                .map_err(loader_error)?;
            return if failed_publications == 0 {
                Ok(ForkTraceLookup::Unbound)
            } else {
                Ok(ForkTraceLookup::IntegrityFailure {
                    failed_publications,
                })
            };
        };
        let clock_ticks_per_second = self.clock_ticks_per_second.ok_or_else(|| {
            CollectorError::new(
                "fork_trace_identity",
                "sysconf(_SC_CLK_TCK) did not return a positive value",
            )
        })?;
        KernelForkTraceBinding::from_runtime(kernel_tgid, binding, clock_ticks_per_second)
            .map(ForkTraceLookup::Bound)
    }

    pub fn bind_launch_trace(
        &mut self,
        request: &TraceBindingRequest,
        pidfd: OwnedFd,
    ) -> Result<TraceBindingHandle, CollectorError> {
        if let Some(reason) = &self.probe_result.reason_unavailable {
            return Err(CollectorError::new("bind_launch_trace", reason.clone()));
        }
        if self.pending_launches.contains_key(&request.trace_id) {
            return Err(CollectorError::new(
                "bind_launch_trace",
                format!("trace {} already has a pending launch", request.trace_id),
            ));
        }
        if let Some(unsupported_required) = request.requested_capabilities.iter().find(|request| {
            !supported_required_capability(
                &request.capability,
                self.loader.config(),
                self.loader.payload_config(),
            ) && request.mode == RequestMode::Required
        }) {
            return Err(CollectorError::new(
                "bind_launch_trace",
                format!(
                    "current libbpf-rs collector path does not support required capability {:?}",
                    unsupported_required.capability
                ),
            ));
        }

        self.ensure_runtime_for_requests(&request.requested_capabilities)?;
        let generation = kernel_start_time(&request.root_observation)?;
        let observer_tgid = request
            .root_observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| CollectorError::new("observer_tgid", "observer TGID is missing"))?;
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
        let armed_binding = match self.runtime_ref()?.arm_launch_binding(
            pidfd,
            observer_tgid,
            request.trace_id,
            generation,
            &request.initial_suppressed_fds,
        ) {
            Ok(armed) => armed,
            Err(error) => {
                let _ = self
                    .runtime_ref()?
                    .unregister_trace_pid_namespace(request.trace_id);
                return Err(loader_error(error));
            }
        };

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
        let root_working_directory = request
            .root_observation
            .host
            .as_ref()
            .and_then(|host| crate::procfs::read_process_cwd(host.pid));
        self.pending_launches.insert(
            request.trace_id,
            PendingLaunchBinding {
                root_identity: request.root_identity,
                root_observation: request.root_observation.clone(),
                generation,
                initial_suppressed_fds: request.initial_suppressed_fds.clone(),
                root_working_directory,
                armed_binding,
            },
        );
        Ok(TraceBindingHandle {
            collector: self.probe_result.descriptor.clone(),
            bound_at: SystemTime::now(),
        })
    }

    pub fn seed_trace_memberships(
        &mut self,
        trace_id: TraceId,
        records: impl IntoIterator<Item = ProcessRecord>,
    ) -> Result<(), CollectorError> {
        let mut seeds = Vec::new();
        for record in records {
            if self
                .pending_launches
                .get(&trace_id)
                .is_some_and(|pending| pending.root_identity == record.identity)
            {
                continue;
            }
            let observation = observation_from_record(&record)?;
            let host = observation.host.as_ref().ok_or_else(|| {
                CollectorError::new("observer_tgid", "observer identity is missing")
            })?;
            let observer_tgid = host.pid;
            let start_time_ticks = host.start_time_ticks;
            seeds.push((observation, observer_tgid, start_time_ticks));
        }
        let requests = seeds
            .iter()
            .map(
                |(_, observer_tgid, start_time_ticks)| ProcessIdentityResolutionRequest {
                    observer_tgid: *observer_tgid,
                    start_time_ticks: *start_time_ticks,
                },
            )
            .collect::<Vec<_>>();
        let resolutions = self
            .runtime_mut()?
            .resolve_process_identities(&requests)
            .map_err(loader_error)?;
        for ((observation, observer_tgid, _), resolution) in seeds.into_iter().zip(resolutions) {
            self.runtime_mut()?
                .track_pid(
                    resolution.kernel_tgid,
                    resolution.start_boottime_ns,
                    observer_tgid,
                    trace_id,
                )
                .map_err(loader_error)?;
            self.file_tracker.seed_process(
                trace_id,
                observation.clone(),
                observation
                    .host
                    .as_ref()
                    .and_then(|host| crate::procfs::read_process_cwd(host.pid)),
            );
            self.bindings.track_with_kernel_tgid(
                trace_id,
                observation,
                resolution.kernel_tgid,
                resolution.start_boottime_ns,
            );
        }
        Ok(())
    }

    pub fn stop_tracking_process(&mut self, pid: u32) -> Result<(), CollectorError> {
        let tracked = self.bindings.remove_pid(pid);
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.untrack_fork_host_pid(pid).map_err(loader_error)?;
            if let Some(tracked) = tracked.as_ref() {
                let kernel_tgid = tracked.kernel_tgid;
                runtime
                    .sweep_suppressed_fds_for_process(kernel_tgid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .unmark_file_bulk_read_fast_process(kernel_tgid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .sweep_file_bulk_read_fast_fds_for_process(
                        kernel_tgid,
                        tracked.kernel_start_time,
                    )
                    .map_err(loader_error)?;
                cleanup_suppressed_fds_for_pid(runtime, &mut self.suppressed_fds, kernel_tgid)?;
                runtime.untrack_pid(kernel_tgid).map_err(loader_error)?;
            }
        }
        Ok(())
    }

    pub fn stop_kernel_tracking_process(&mut self, pid: u32) -> Result<(), CollectorError> {
        if let Some(trace_id) = self
            .pending_launches
            .iter()
            .find_map(|(trace_id, pending)| {
                pending
                    .root_observation
                    .host
                    .as_ref()
                    .is_some_and(|host| host.pid == pid)
                    .then_some(*trace_id)
            })
        {
            self.cancel_pending_launch(trace_id)?;
        }
        let tracked = self.bindings.by_host_pid(pid).cloned();
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.untrack_fork_host_pid(pid).map_err(loader_error)?;
            if let Some(tracked) = tracked.as_ref() {
                let kernel_tgid = tracked.kernel_tgid;
                runtime
                    .sweep_suppressed_fds_for_process(kernel_tgid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .unmark_file_bulk_read_fast_process(kernel_tgid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .sweep_file_bulk_read_fast_fds_for_process(
                        kernel_tgid,
                        tracked.kernel_start_time,
                    )
                    .map_err(loader_error)?;
                cleanup_suppressed_fds_for_pid(runtime, &mut self.suppressed_fds, kernel_tgid)?;
                runtime.untrack_pid(kernel_tgid).map_err(loader_error)?;
            }
        }
        Ok(())
    }

    pub(super) fn register_initial_suppressed_fds(
        &mut self,
        trace_id: TraceId,
        root_start_time: u64,
        root_kernel_tgid: u32,
        initial_fds: &[InitialSuppressedFd],
    ) -> Result<(), CollectorError> {
        for initial in initial_fds {
            let map_identity = KernelProcessCoordinates {
                pid: root_kernel_tgid,
                start_time: root_start_time,
            };
            let fd = ProcessSuppressedFd {
                process: map_identity,
                fd: initial.fd,
                purpose: initial.purpose,
            };
            self.runtime_mut()?
                .suppress_fd(trace_id, &fd)
                .map_err(loader_error)?;
            self.suppressed_fds.push(TraceSuppressedFd { trace_id, fd });
        }
        Ok(())
    }

    pub(super) fn register_trace_pid_namespace(
        &mut self,
        trace_id: TraceId,
        observation: &ProcessObservation,
    ) -> Result<(), CollectorError> {
        self.runtime_mut()?
            .register_trace_pid_namespace(
                trace_id,
                observation
                    .host
                    .as_ref()
                    .map(|host| host.pid)
                    .ok_or_else(|| CollectorError::new("host_pid", "root host PID is missing"))?,
            )
            .map_err(loader_error)?;
        Ok(())
    }

    pub(super) fn cleanup_suppressed_fds_for_process(
        &mut self,
        pid: u32,
        generation: u64,
    ) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            self.suppressed_fds.retain(|entry| {
                entry.fd.process.pid != pid || entry.fd.process.start_time != generation
            });
            return Ok(());
        };
        cleanup_suppressed_fds_for_process(runtime, &mut self.suppressed_fds, pid, generation)
    }

    pub(super) fn cancel_pending_launch(
        &mut self,
        trace_id: TraceId,
    ) -> Result<bool, CollectorError> {
        let Some(pending) = self.pending_launches.get(&trace_id) else {
            return Ok(false);
        };
        let deleted = self
            .runtime_ref()?
            .cancel_launch_binding(&pending.armed_binding)
            .map_err(loader_error)?;
        self.pending_launches.remove(&trace_id);
        Ok(deleted)
    }
}

fn cleanup_suppressed_fds_for_pid(
    runtime: &EbpfRuntime,
    registry: &mut Vec<TraceSuppressedFd>,
    pid: u32,
) -> Result<(), CollectorError> {
    let mut retained = Vec::new();
    for entry in std::mem::take(registry) {
        if entry.fd.process.pid == pid {
            runtime
                .unsuppress_fd(&entry.fd.process, entry.fd.fd)
                .map_err(loader_error)?;
        } else {
            retained.push(entry);
        }
    }
    *registry = retained;
    Ok(())
}

pub(super) fn cleanup_suppressed_fds_for_process(
    runtime: &EbpfRuntime,
    registry: &mut Vec<TraceSuppressedFd>,
    pid: u32,
    generation: u64,
) -> Result<(), CollectorError> {
    runtime
        .sweep_suppressed_fds_for_process(pid, generation)
        .map_err(loader_error)?;
    let mut retained = Vec::new();
    for entry in std::mem::take(registry) {
        if entry.fd.process.pid == pid && entry.fd.process.start_time == generation {
            runtime
                .unsuppress_fd(&entry.fd.process, entry.fd.fd)
                .map_err(loader_error)?;
        } else {
            retained.push(entry);
        }
    }
    *registry = retained;
    Ok(())
}

pub(super) fn cleanup_suppressed_fds_for_trace(
    runtime: &EbpfRuntime,
    registry: &mut Vec<TraceSuppressedFd>,
    trace_id: TraceId,
) -> Result<(), CollectorError> {
    runtime
        .sweep_suppressed_fds_for_trace(trace_id)
        .map_err(loader_error)?;
    let mut retained = Vec::new();
    for entry in std::mem::take(registry) {
        if entry.trace_id == trace_id {
            runtime
                .unsuppress_fd(&entry.fd.process, entry.fd.fd)
                .map_err(loader_error)?;
        } else {
            retained.push(entry);
        }
    }
    *registry = retained;
    Ok(())
}

fn observation_from_record(record: &ProcessRecord) -> Result<ProcessObservation, CollectorError> {
    let host = record.host.clone().ok_or_else(|| {
        CollectorError::new(
            "process_record",
            format!("process {} has no host coordinates", record.identity.get()),
        )
    })?;
    let namespace = record.namespaces.iter().next().cloned().ok_or_else(|| {
        CollectorError::new(
            "process_record",
            format!(
                "process {} has no namespace coordinates",
                record.identity.get()
            ),
        )
    })?;
    Ok(ProcessObservation::host(host).with_namespace(namespace))
}

pub(super) fn kernel_start_time(observation: &ProcessObservation) -> Result<u64, CollectorError> {
    let host = observation
        .host
        .as_ref()
        .ok_or_else(|| CollectorError::new("process_start_time", "host coordinates are missing"))?;
    let start_time = host.start_boottime_ns.unwrap_or(host.start_time_ticks);
    if start_time == 0 {
        return Err(CollectorError::new(
            "process_start_time",
            "kernel process start time is missing",
        ));
    }
    Ok(start_time)
}

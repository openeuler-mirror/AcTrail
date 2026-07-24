//! eBPF-backed collector adapter.

pub mod capability_probe;
#[path = "collector/dynamic_go_tls.rs"]
mod collector_dynamic_go_tls;
#[path = "collector/events.rs"]
mod collector_events;
pub mod decode;
pub mod loader;
pub mod maps;
pub mod procfs;
pub mod sensors;

use std::os::fd::RawFd;
use std::time::SystemTime;

use collector_binding::{
    CoverageGuardHandle, CoverageGuardRequest, TraceBindingHandle, TraceBindingRequest,
};
use collector_instance::{CollectorError, CollectorInstance, CollectorPollBatch};
use collector_stats::{CollectorStats, DropCounter};
use config_core::daemon::{EbpfCollectorConfig, FileBulkReadFastPathConfig, PayloadConfig};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{
    InitialSuppressedFd, KernelProcessCoordinates, ProcessObservation, ProcessRecord,
    ProcessSuppressedFd,
};

use crate::capability_probe::{EbpfProbeResult, probe};
use crate::decode::FileTracker;
pub use crate::decode::{
    SOCKET_PAYLOAD_DIRECTION_INBOUND, SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
    SOCKET_PAYLOAD_SYSCALL_READ, SOCKET_PAYLOAD_SYSCALL_RECVFROM, SOCKET_PAYLOAD_SYSCALL_SENDMSG,
    SOCKET_PAYLOAD_SYSCALL_SENDTO, SOCKET_PAYLOAD_SYSCALL_WRITE, SOCKET_PAYLOAD_SYSCALL_WRITEV,
    SocketPayloadCompletion, TlsDiagnosticEvent, TlsPayloadCaptureRequest, TlsPayloadCompletion,
    TlsPayloadDirectCapture,
};
use crate::loader::{
    AttachPlan, EbpfProgramLoader, EbpfRuntime, LoaderError, PendingTlsPayloadOp,
    TlsPayloadDiagnostics,
};
use crate::maps::BindingStateMap;
use collector_dynamic_go_tls::DynamicGoTlsAttacher;

#[cfg(test)]
mod tests;

pub struct EbpfCollector {
    probe_result: EbpfProbeResult,
    loader: EbpfProgramLoader,
    bindings: BindingStateMap,
    runtime: Option<EbpfRuntime>,
    file_tracker: FileTracker,
    dynamic_go_tls: DynamicGoTlsAttacher,
    file_bulk_read_fast_path: FileBulkReadFastPathConfig,
    tls_capture_requests: Vec<TlsPayloadCaptureRequest>,
    tls_completions: Vec<TlsPayloadCompletion>,
    tls_direct_captures: Vec<TlsPayloadDirectCapture>,
    tls_diagnostic_events: Vec<TlsDiagnosticEvent>,
    socket_completions: Vec<SocketPayloadCompletion>,
    suppressed_fds: Vec<TraceSuppressedFd>,
    binding_gap_drops: u64,
    binding_gap_lifecycle_skips: u64,
    clock_ticks_per_second: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TraceSuppressedFd {
    trace_id: TraceId,
    fd: ProcessSuppressedFd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EbpfCollectorDebugSnapshot {
    pub active_binding_traces: usize,
    pub attached_programs: Vec<String>,
    pub last_raw_sample_count: usize,
    pub tracked_trace_id: Option<TraceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForkTraceLookup {
    Unavailable,
    Unbound,
    Bound(KernelForkTraceBinding),
    IntegrityFailure { failed_publications: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelForkTraceBinding {
    trace_id: TraceId,
    host_pid: u32,
    start_boottime_ns: u64,
    start_time_ticks: u64,
}

impl KernelForkTraceBinding {
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub fn validate_and_enrich(
        &self,
        mut observation: ProcessObservation,
    ) -> Result<ProcessObservation, CollectorError> {
        let host = observation.host.as_mut().ok_or_else(|| {
            CollectorError::new(
                "fork_trace_identity",
                "procfs observation has no host coordinates",
            )
        })?;
        if host.pid != self.host_pid || host.start_time_ticks != self.start_time_ticks {
            return Err(CollectorError::new(
                "fork_trace_identity",
                format!(
                    "kernel fork binding generation mismatch for host PID {}",
                    self.host_pid
                ),
            ));
        }
        host.start_boottime_ns = Some(self.start_boottime_ns);
        Ok(observation)
    }
}

impl EbpfCollector {
    pub fn new(
        config: EbpfCollectorConfig,
        payload_config: PayloadConfig,
        file_bulk_read_fast_path: FileBulkReadFastPathConfig,
    ) -> Self {
        let mut probe_result = probe();
        if !config.enabled {
            probe_result.reason_unavailable =
                Some("collector disabled by configuration".to_string());
        }
        Self {
            probe_result: probe_result_for_config(probe_result, &payload_config),
            loader: EbpfProgramLoader::new(
                config,
                payload_config.clone(),
                file_bulk_read_fast_path.clone(),
            ),
            bindings: BindingStateMap::default(),
            runtime: None,
            file_tracker: FileTracker::default(),
            dynamic_go_tls: DynamicGoTlsAttacher::new(&payload_config.tls),
            file_bulk_read_fast_path,
            tls_capture_requests: Vec::new(),
            tls_completions: Vec::new(),
            tls_direct_captures: Vec::new(),
            tls_diagnostic_events: Vec::new(),
            socket_completions: Vec::new(),
            suppressed_fds: Vec::new(),
            binding_gap_drops: 0,
            binding_gap_lifecycle_skips: 0,
            clock_ticks_per_second: clock_ticks_per_second(),
        }
    }

    pub fn fork_trace_lookup(&self, host_pid: u32) -> Result<ForkTraceLookup, CollectorError> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(ForkTraceLookup::Unavailable);
        };
        let binding = runtime.fork_trace_binding(host_pid).map_err(loader_error)?;
        let Some(binding) = binding else {
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
        let start_time_ticks = u64::try_from(
            u128::from(binding.child_start_boottime_ns)
                .saturating_mul(u128::from(clock_ticks_per_second))
                / 1_000_000_000_u128,
        )
        .map_err(|_| {
            CollectorError::new(
                "fork_trace_identity",
                "fork start generation does not fit procfs clock ticks",
            )
        })?;
        Ok(ForkTraceLookup::Bound(KernelForkTraceBinding {
            trace_id: binding.trace_id,
            host_pid,
            start_boottime_ns: binding.child_start_boottime_ns,
            start_time_ticks,
        }))
    }

    pub fn probe_result(&self) -> &EbpfProbeResult {
        &self.probe_result
    }

    pub fn preflight_capability_requests(
        &self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        if let Some(reason) = &self.probe_result.reason_unavailable {
            return Err(CollectorError::new("ebpf_preflight", reason.clone()));
        }

        let requests = requests
            .iter()
            .filter(|request| request.mode != RequestMode::Disabled)
            .filter(|request| {
                self.probe_result
                    .descriptor
                    .capabilities
                    .iter()
                    .any(|descriptor| descriptor.capability == request.capability)
            })
            .cloned()
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return Err(CollectorError::new(
                "ebpf_preflight",
                "capture profile requests host eBPF observation, but no requested capability is exposed by the eBPF collector descriptor",
            ));
        }
        if let Some(unsupported_required) = requests.iter().find(|request| {
            !supported_required_capability(&request.capability, self.loader.payload_config())
                && request.mode == RequestMode::Required
        }) {
            return Err(CollectorError::new(
                "ebpf_preflight",
                format!(
                    "current libbpf-rs collector path does not support required capability {:?}",
                    unsupported_required.capability
                ),
            ));
        }

        let attach_plan = AttachPlan::from_requests(
            &requests,
            self.loader.config(),
            self.loader.payload_config(),
        );
        let runtime = self
            .loader
            .load_runtime_with_plan(&attach_plan)
            .map_err(loader_error)?;
        if let Some(missing) = requests.iter().find(|request| {
            request.mode == RequestMode::Required
                && !runtime
                    .attached_capabilities()
                    .contains(&request.capability)
        }) {
            return Err(CollectorError::new(
                "ebpf_preflight",
                format!(
                    "preflight loaded eBPF runtime without required capability {:?}",
                    missing.capability
                ),
            ));
        }
        Ok(())
    }

    pub fn seed_trace_memberships(
        &mut self,
        trace_id: TraceId,
        records: impl IntoIterator<Item = ProcessRecord>,
    ) -> Result<(), CollectorError> {
        for record in records {
            let observation = observation_from_record(&record)?;
            let map_pid = self.map_pid_for_observation(&observation)?;
            let kernel_start_time = kernel_start_time(&observation)?;
            self.runtime_mut()?
                .track_pid(map_pid, kernel_start_time, trace_id)
                .map_err(loader_error)?;
            self.file_tracker.seed_process(
                trace_id,
                observation.clone(),
                observation
                    .host
                    .as_ref()
                    .and_then(|host| crate::procfs::read_process_cwd(host.pid)),
            );
            self.bindings
                .track_with_map_pid(trace_id, observation, map_pid, kernel_start_time);
        }
        Ok(())
    }

    pub fn stop_tracking_process(&mut self, pid: u32) -> Result<(), CollectorError> {
        let tracked = self.bindings.by_host_pid(pid).cloned();
        let map_pid = self
            .bindings
            .remove_pid(pid)
            .map(|tracked| tracked.map_pid)
            .unwrap_or(pid);
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.untrack_fork_host_pid(pid).map_err(loader_error)?;
            if let Some(tracked) = tracked.as_ref() {
                runtime
                    .sweep_suppressed_fds_for_process(map_pid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .unmark_file_bulk_read_fast_process(map_pid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .sweep_file_bulk_read_fast_fds_for_process(map_pid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
            }
            cleanup_suppressed_fds_for_pid(runtime, &mut self.suppressed_fds, map_pid)?;
            runtime.untrack_pid(map_pid).map_err(loader_error)?;
        }
        Ok(())
    }

    pub fn stop_kernel_tracking_process(&mut self, pid: u32) -> Result<(), CollectorError> {
        let tracked = self.bindings.by_host_pid(pid).cloned();
        let map_pid = tracked
            .as_ref()
            .map(|tracked| tracked.map_pid)
            .unwrap_or(pid);
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.untrack_fork_host_pid(pid).map_err(loader_error)?;
            if let Some(tracked) = tracked.as_ref() {
                runtime
                    .sweep_suppressed_fds_for_process(map_pid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .unmark_file_bulk_read_fast_process(map_pid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
                runtime
                    .sweep_file_bulk_read_fast_fds_for_process(map_pid, tracked.kernel_start_time)
                    .map_err(loader_error)?;
            }
            cleanup_suppressed_fds_for_pid(runtime, &mut self.suppressed_fds, map_pid)?;
            runtime.untrack_pid(map_pid).map_err(loader_error)?;
        }
        Ok(())
    }

    pub fn event_poll_fd(&self) -> Result<Option<RawFd>, CollectorError> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.event_poll_fd().map_err(loader_error))
            .transpose()
    }

    pub fn lookup_pending_tls_payload_op(
        &self,
        tid: u32,
    ) -> Result<Option<PendingTlsPayloadOp>, CollectorError> {
        self.runtime_ref()?
            .lookup_pending_tls_payload_op(tid)
            .map_err(loader_error)
    }

    pub fn lookup_socket_fd_generation(
        &self,
        pid: u32,
        fd: u32,
    ) -> Result<Option<u32>, CollectorError> {
        self.runtime_ref()?
            .lookup_socket_fd_generation(pid, fd)
            .map_err(loader_error)
    }

    pub fn take_tls_completions(&mut self) -> Vec<TlsPayloadCompletion> {
        std::mem::take(&mut self.tls_completions)
    }

    pub fn take_tls_capture_requests(&mut self) -> Vec<TlsPayloadCaptureRequest> {
        std::mem::take(&mut self.tls_capture_requests)
    }

    pub fn take_tls_direct_captures(&mut self) -> Vec<TlsPayloadDirectCapture> {
        std::mem::take(&mut self.tls_direct_captures)
    }

    pub fn take_tls_diagnostic_events(&mut self) -> Vec<TlsDiagnosticEvent> {
        std::mem::take(&mut self.tls_diagnostic_events)
    }

    pub fn take_socket_completions(&mut self) -> Vec<SocketPayloadCompletion> {
        std::mem::take(&mut self.socket_completions)
    }

    pub fn tls_payload_diagnostics(&self) -> Result<Option<TlsPayloadDiagnostics>, CollectorError> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.tls_payload_diagnostics().map_err(loader_error))
            .transpose()
    }

    pub fn take_event_transport_loss_summaries(&mut self) -> Vec<String> {
        self.runtime
            .as_mut()
            .map(EbpfRuntime::take_event_transport_loss_summaries)
            .unwrap_or_default()
    }

    pub fn flush_transport(&mut self) -> Result<(), CollectorError> {
        self.runtime
            .as_mut()
            .map(EbpfRuntime::flush_transport)
            .transpose()
            .map(|_| ())
            .map_err(loader_error)
    }

    pub fn debug_snapshot_for_pid(
        &self,
        pid: u32,
    ) -> Result<EbpfCollectorDebugSnapshot, CollectorError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| CollectorError::new("runtime", "eBPF runtime was not initialized"))?;
        Ok(EbpfCollectorDebugSnapshot {
            active_binding_traces: self.bindings.trace_count(),
            attached_programs: runtime.attached_programs().to_vec(),
            last_raw_sample_count: runtime.last_raw_sample_count(),
            tracked_trace_id: runtime
                .tracked_trace_id(
                    self.bindings
                        .by_host_pid(pid)
                        .map(|tracked| tracked.map_pid)
                        .unwrap_or(pid),
                )
                .map_err(loader_error)?,
        })
    }

    fn ensure_runtime_for_requests(
        &mut self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        let attach_plan =
            AttachPlan::from_requests(requests, self.loader.config(), self.loader.payload_config());
        if self.idle_runtime_needs_replan(&attach_plan) {
            self.runtime = None;
        }
        if self.runtime.is_none() {
            let runtime = self
                .loader
                .load_runtime_with_plan(&attach_plan)
                .map_err(loader_error)?;
            self.runtime = Some(runtime);
        }
        self.ensure_required_capabilities_attached(requests)
    }

    fn idle_runtime_needs_replan(&self, attach_plan: &AttachPlan) -> bool {
        self.bindings.trace_count() == 0
            && self.runtime.as_ref().is_some_and(|runtime| {
                !attach_plan.is_satisfied_by(runtime.attached_capabilities())
            })
    }

    fn ensure_required_capabilities_attached(
        &self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        let runtime = self.runtime_ref()?;
        if let Some(missing) = requests.iter().find(|request| {
            request.mode == RequestMode::Required
                && !runtime
                    .attached_capabilities()
                    .contains(&request.capability)
        }) {
            return Err(CollectorError::new(
                "bind_trace",
                format!(
                    "active eBPF runtime is attached without required capability {:?}; finish active traces and bind again with the requested capability set, or restart the daemon",
                    missing.capability
                ),
            ));
        }
        Ok(())
    }

    fn runtime_ref(&self) -> Result<&EbpfRuntime, CollectorError> {
        self.runtime
            .as_ref()
            .ok_or_else(|| CollectorError::new("runtime", "eBPF runtime was not initialized"))
    }

    fn runtime_mut(&mut self) -> Result<&mut EbpfRuntime, CollectorError> {
        self.runtime
            .as_mut()
            .ok_or_else(|| CollectorError::new("runtime", "eBPF runtime was not initialized"))
    }

    fn register_initial_suppressed_fds(
        &mut self,
        trace_id: TraceId,
        root_start_time: u64,
        root_map_pid: u32,
        initial_fds: &[InitialSuppressedFd],
    ) -> Result<(), CollectorError> {
        for initial in initial_fds {
            let map_identity = KernelProcessCoordinates {
                pid: root_map_pid,
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

    fn register_trace_pid_namespace(
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

    fn map_pid_for_observation(
        &self,
        observation: &ProcessObservation,
    ) -> Result<u32, CollectorError> {
        observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| CollectorError::new("host_pid", "host PID is missing"))
    }

    fn cleanup_suppressed_fds_for_process(
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

fn cleanup_suppressed_fds_for_process(
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

fn cleanup_suppressed_fds_for_trace(
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

fn kernel_start_time(observation: &ProcessObservation) -> Result<u64, CollectorError> {
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
            !supported_required_capability(&request.capability, self.loader.payload_config())
                && request.mode == RequestMode::Required
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
        CollectorStats {
            collector_name: CollectorName::new("ebpf"),
            active_bindings: self.bindings.trace_count(),
            last_heartbeat_at: SystemTime::now(),
            dropped,
        }
    }
}

fn loader_error(error: LoaderError) -> CollectorError {
    CollectorError::new(error.stage, error.message)
}

fn clock_ticks_per_second() -> Option<u64> {
    let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (value > 0).then_some(value as u64)
}

fn probe_result_for_config(
    mut result: EbpfProbeResult,
    payload: &PayloadConfig,
) -> EbpfProbeResult {
    if payload.tls.enabled && !payload.tls.capture_backend.is_sync() {
        result
            .descriptor
            .capabilities
            .push(model_core::capability::CapabilityDescriptor::new(
                Capability::TlsPlaintextPayload,
                vec![model_core::capability::CapabilityField::new(
                    "tls_plaintext_segment",
                    model_core::capability::GuaranteeClass::RequiresPayloadCollector,
                )],
            ));
    }
    if stdio_payload_capability_configured(payload) {
        result
            .descriptor
            .capabilities
            .push(model_core::capability::CapabilityDescriptor::new(
                Capability::StdioChunk,
                vec![model_core::capability::CapabilityField::new(
                    "stdin_stdout_stderr_segment",
                    model_core::capability::GuaranteeClass::RequiresPayloadCollector,
                )],
            ));
    }
    if payload.socket.enabled {
        result
            .descriptor
            .capabilities
            .push(model_core::capability::CapabilityDescriptor::new(
                Capability::SocketPlaintextPayload,
                vec![model_core::capability::CapabilityField::new(
                    "socket_plaintext_segment",
                    model_core::capability::GuaranteeClass::RequiresPayloadCollector,
                )],
            ));
    }
    result
}

fn supported_required_capability(capability: &Capability, payload: &PayloadConfig) -> bool {
    matches!(
        capability,
        Capability::ProcLifecycle
            | Capability::NetTransport
            | Capability::FsAccessBasic
            | Capability::FsMmap
            | Capability::IpcPipeFifo
            | Capability::IpcUnixSocket
    ) || (matches!(capability, Capability::TlsPlaintextPayload)
        && payload.tls.enabled
        && !payload.tls.capture_backend.is_sync())
        || (matches!(capability, Capability::SocketPlaintextPayload) && payload.socket.enabled)
        || (matches!(capability, Capability::StdioChunk)
            && stdio_payload_capability_configured(payload))
}

fn stdio_payload_capability_configured(payload: &PayloadConfig) -> bool {
    payload.stdio.enabled
        && (payload.stdio.capture_stdin
            || payload.stdio.capture_stdout
            || payload.stdio.capture_stderr)
}

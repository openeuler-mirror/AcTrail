//! eBPF-backed collector adapter.

pub mod capability_probe;
#[path = "collector/dynamic_go_tls.rs"]
mod collector_dynamic_go_tls;
#[path = "collector/events.rs"]
mod collector_events;
pub mod decode;
pub mod loader;
pub mod maps;
mod process_context;
pub mod procfs;
pub mod sensors;

use std::os::fd::RawFd;
use std::time::SystemTime;

use collector_binding::{
    CoverageGuardHandle, CoverageGuardRequest, TraceBindingHandle, TraceBindingRequest,
};
use collector_instance::{CollectorError, CollectorInstance, CollectorPollBatch};
use collector_stats::{CollectorStats, DropCounter};
use config_core::daemon::{EbpfCollectorConfig, PayloadConfig};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{
    InitialSuppressedFd, NamespaceIdentity, ProcessIdentity, ProcessSuppressedFd,
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
use crate::procfs::{ProcfsIdentityReader, read_process_namespace_pid};
use collector_dynamic_go_tls::DynamicGoTlsAttacher;

#[cfg(test)]
mod tests {
    const TLS_PAYLOAD_BPF: &str = include_str!("../bpf/actrail_tls_payload.h");

    #[test]
    fn bpf_copy_seccomp_fallback_requests_seccomp_capture_when_direct_copy_misses() {
        let (_, after_fallback_backend_check) = TLS_PAYLOAD_BPF
            .split_once(
                "payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK",
            )
            .expect("BPF copy/seccomp fallback branch");
        let (fallback_block, _) = after_fallback_backend_check
            .split_once("payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ")
            .expect("seccomp user-read branch");

        assert!(
            fallback_block.contains("emit_tls_capture_request(&op, tgid, tid, op.requested_size)"),
            "BPF copy/seccomp fallback must emit a capture request after direct-copy misses"
        );
    }
}

pub struct EbpfCollector {
    probe_result: EbpfProbeResult,
    loader: EbpfProgramLoader,
    bindings: BindingStateMap,
    runtime: Option<EbpfRuntime>,
    identity_reader: ProcfsIdentityReader,
    file_tracker: FileTracker,
    dynamic_go_tls: DynamicGoTlsAttacher,
    tls_capture_requests: Vec<TlsPayloadCaptureRequest>,
    tls_completions: Vec<TlsPayloadCompletion>,
    tls_direct_captures: Vec<TlsPayloadDirectCapture>,
    tls_diagnostic_events: Vec<TlsDiagnosticEvent>,
    socket_completions: Vec<SocketPayloadCompletion>,
    suppressed_fds: Vec<TraceSuppressedFd>,
    active_pid_namespace: Option<NamespaceIdentity>,
    binding_gap_drops: u64,
    binding_gap_lifecycle_skips: u64,
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

impl EbpfCollector {
    pub fn new(config: EbpfCollectorConfig, payload_config: PayloadConfig) -> Self {
        let mut probe_result = probe();
        if !config.enabled {
            probe_result.reason_unavailable =
                Some("collector disabled by configuration".to_string());
        }
        Self {
            probe_result: probe_result_for_config(probe_result, &payload_config),
            loader: EbpfProgramLoader::new(config, payload_config.clone()),
            bindings: BindingStateMap::default(),
            runtime: None,
            identity_reader: ProcfsIdentityReader,
            file_tracker: FileTracker::default(),
            dynamic_go_tls: DynamicGoTlsAttacher::new(&payload_config.tls),
            tls_capture_requests: Vec::new(),
            tls_completions: Vec::new(),
            tls_direct_captures: Vec::new(),
            tls_diagnostic_events: Vec::new(),
            socket_completions: Vec::new(),
            suppressed_fds: Vec::new(),
            active_pid_namespace: None,
            binding_gap_drops: 0,
            binding_gap_lifecycle_skips: 0,
        }
    }

    pub fn probe_result(&self) -> &EbpfProbeResult {
        &self.probe_result
    }

    pub fn seed_trace_memberships(
        &mut self,
        trace_id: TraceId,
        identities: impl IntoIterator<Item = ProcessIdentity>,
    ) -> Result<(), CollectorError> {
        for identity in identities {
            let map_pid = self.map_pid_for_identity(&identity)?;
            self.runtime_mut()?
                .track_pid(map_pid, &identity, trace_id)
                .map_err(loader_error)?;
            self.file_tracker.seed_process(
                trace_id,
                identity.clone(),
                crate::procfs::read_process_cwd(identity.pid),
            );
            self.bindings
                .track_with_map_pid(trace_id, identity, map_pid);
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
            if let Some(tracked) = tracked.as_ref() {
                runtime
                    .sweep_suppressed_fds_for_process(map_pid, tracked.identity.generation)
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
            if let Some(tracked) = tracked.as_ref() {
                runtime
                    .sweep_suppressed_fds_for_process(map_pid, tracked.identity.generation)
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
        root_identity: &ProcessIdentity,
        root_map_pid: u32,
        initial_fds: &[InitialSuppressedFd],
    ) -> Result<(), CollectorError> {
        for initial in initial_fds {
            let map_identity = ProcessIdentity {
                pid: root_map_pid,
                ..root_identity.clone()
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

    fn ensure_active_pid_namespace(
        &mut self,
        identity: &ProcessIdentity,
    ) -> Result<(), CollectorError> {
        let namespace = identity.pid_namespace.clone().ok_or_else(|| {
            CollectorError::new("pid_namespace", "root process has no PID namespace")
        })?;
        if self.bindings.trace_count() > 0
            && self
                .active_pid_namespace
                .as_ref()
                .is_some_and(|active| active != &namespace)
        {
            return Err(CollectorError::new(
                "pid_namespace",
                format!(
                    "active eBPF runtime is already configured for {}; finish active traces before binding {}",
                    self.active_pid_namespace
                        .as_ref()
                        .map(NamespaceIdentity::as_str)
                        .unwrap_or("unknown"),
                    namespace.as_str()
                ),
            ));
        }
        self.runtime_mut()?
            .configure_pid_namespace_for_pid(identity.pid)
            .map_err(loader_error)?;
        self.active_pid_namespace = Some(namespace);
        Ok(())
    }

    fn map_pid_for_identity(&self, identity: &ProcessIdentity) -> Result<u32, CollectorError> {
        read_process_namespace_pid(identity.pid)
            .map_err(|error| CollectorError::new("pid_namespace", error))
    }

    fn cleanup_suppressed_fds_for_process(
        &mut self,
        pid: u32,
        generation: u64,
    ) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            self.suppressed_fds.retain(|entry| {
                entry.fd.process.pid != pid || entry.fd.process.generation != generation
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
        if entry.fd.process.pid == pid && entry.fd.process.generation == generation {
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
        self.ensure_active_pid_namespace(&request.root_identity)?;
        let attached_capabilities = self.runtime_ref()?.attached_capabilities().clone();
        let root_map_pid = request.root_namespace_pid;
        let runtime = self.runtime_mut()?;
        runtime
            .track_pid(root_map_pid, &request.root_identity, request.trace_id)
            .map_err(loader_error)?;
        if let Err(error) = self.register_initial_suppressed_fds(
            request.trace_id,
            &request.root_identity,
            root_map_pid,
            &request.initial_suppressed_fds,
        ) {
            let _ = self
                .runtime_mut()
                .and_then(|runtime| runtime.untrack_pid(root_map_pid).map_err(loader_error));
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
        self.bindings.set_trace_pid_namespace(
            request.trace_id,
            request.root_identity.pid_namespace.clone().ok_or_else(|| {
                CollectorError::new("pid_namespace", "root process has no PID namespace")
            })?,
        );
        self.bindings.track_with_map_pid(
            request.trace_id,
            request.root_identity.clone(),
            root_map_pid,
        );
        self.file_tracker.seed_process(
            request.trace_id,
            request.root_identity.clone(),
            crate::procfs::read_process_cwd(request.root_identity.pid),
        );
        Ok(TraceBindingHandle {
            collector: self.probe_result.descriptor.clone(),
            bound_at: SystemTime::now(),
        })
    }

    fn unbind_trace(&mut self, trace_id: TraceId) -> Result<(), CollectorError> {
        if let Some(runtime) = self.runtime.as_mut() {
            cleanup_suppressed_fds_for_trace(runtime, &mut self.suppressed_fds, trace_id)?;
            for tracked in self.bindings.remove_trace(trace_id) {
                runtime.untrack_pid(tracked.map_pid).map_err(loader_error)?;
            }
        } else {
            let _ = self.bindings.remove_trace(trace_id);
        }
        self.file_tracker.remove_trace(trace_id);
        if self.bindings.trace_count() == 0 {
            self.active_pid_namespace = None;
        }
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

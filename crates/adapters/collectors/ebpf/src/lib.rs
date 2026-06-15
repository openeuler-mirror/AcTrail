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
use collector_stats::CollectorStats;
use config_core::daemon::{EbpfCollectorConfig, PayloadConfig};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{CollectorName, TraceId};
use model_core::process::ProcessIdentity;

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
    AttachPlan, EbpfProgramLoader, EbpfRuntime, LoaderError, PendingTlsPayloadOp, PidNamespace,
    TlsPayloadDiagnostics,
};
use crate::maps::BindingStateMap;
use crate::procfs::{ProcfsIdentityReader, read_pid_namespace_handle};
use collector_dynamic_go_tls::DynamicGoTlsAttacher;

pub struct EbpfCollector {
    probe_result: EbpfProbeResult,
    loader: EbpfProgramLoader,
    bindings: BindingStateMap,
    runtime: Option<EbpfRuntime>,
    identity_reader: ProcfsIdentityReader,
    pid_namespace: Option<PidNamespace>,
    file_tracker: FileTracker,
    dynamic_go_tls: DynamicGoTlsAttacher,
    tls_capture_requests: Vec<TlsPayloadCaptureRequest>,
    tls_completions: Vec<TlsPayloadCompletion>,
    tls_direct_captures: Vec<TlsPayloadDirectCapture>,
    tls_diagnostic_events: Vec<TlsDiagnosticEvent>,
    socket_completions: Vec<SocketPayloadCompletion>,
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
            pid_namespace: None,
            file_tracker: FileTracker::default(),
            dynamic_go_tls: DynamicGoTlsAttacher::new(&payload_config.tls),
            tls_capture_requests: Vec::new(),
            tls_completions: Vec::new(),
            tls_direct_captures: Vec::new(),
            tls_diagnostic_events: Vec::new(),
            socket_completions: Vec::new(),
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
        let mut seeded = Vec::new();
        for identity in identities {
            self.ensure_pid_namespace(identity.pid)?;
            self.runtime_mut()?
                .track_pid(&identity, trace_id)
                .map_err(loader_error)?;
            self.file_tracker.seed_process(identity.pid);
            seeded.push(identity);
        }
        self.bindings.track_many(trace_id, seeded);
        Ok(())
    }

    pub fn stop_tracking_process(&mut self, pid: u32) -> Result<(), CollectorError> {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.untrack_pid(pid).map_err(loader_error)?;
        }
        self.bindings.remove_pid(pid);
        self.file_tracker.remove_process(pid);
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
            tracked_trace_id: runtime.tracked_trace_id(pid).map_err(loader_error)?,
        })
    }

    fn ensure_pid_namespace(&mut self, pid: u32) -> Result<(), CollectorError> {
        let namespace = read_pid_namespace_handle(pid)
            .map_err(|error| CollectorError::new("pid_namespace", error))?;
        if let Some(active_namespace) = self.pid_namespace {
            if active_namespace != namespace {
                return Err(CollectorError::new(
                    "pid_namespace",
                    "current eBPF collector runtime supports one active PID namespace",
                ));
            }
            return Ok(());
        }

        self.runtime_mut()?
            .set_pid_namespace(namespace)
            .map_err(loader_error)?;
        self.pid_namespace = Some(namespace);
        Ok(())
    }

    fn ensure_runtime_for_requests(
        &mut self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        let attach_plan =
            AttachPlan::from_requests(requests, self.loader.config(), self.loader.payload_config());
        if self.idle_runtime_needs_replan(&attach_plan) {
            self.runtime = None;
            self.pid_namespace = None;
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
        self.ensure_pid_namespace(request.root_identity.pid)?;
        let attached_capabilities = self.runtime_ref()?.attached_capabilities().clone();
        let runtime = self.runtime_mut()?;
        runtime
            .track_pid(&request.root_identity, request.trace_id)
            .map_err(loader_error)?;
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
            .track(request.trace_id, request.root_identity.clone());
        self.file_tracker.seed_process(request.root_identity.pid);
        Ok(TraceBindingHandle {
            collector: self.probe_result.descriptor.clone(),
            bound_at: SystemTime::now(),
        })
    }

    fn unbind_trace(&mut self, trace_id: TraceId) -> Result<(), CollectorError> {
        if let Some(runtime) = self.runtime.as_mut() {
            for tracked in self.bindings.remove_trace(trace_id) {
                runtime
                    .untrack_pid(tracked.identity.pid)
                    .map_err(loader_error)?;
                self.file_tracker.remove_process(tracked.identity.pid);
            }
        } else {
            for tracked in self.bindings.remove_trace(trace_id) {
                self.file_tracker.remove_process(tracked.identity.pid);
            }
        }
        Ok(())
    }

    fn poll_batch(&mut self) -> Result<CollectorPollBatch, CollectorError> {
        self.poll_batch_impl()
    }

    fn stats(&self) -> CollectorStats {
        CollectorStats {
            collector_name: CollectorName::new("ebpf"),
            active_bindings: self.bindings.trace_count(),
            last_heartbeat_at: SystemTime::now(),
            dropped: Vec::new(),
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

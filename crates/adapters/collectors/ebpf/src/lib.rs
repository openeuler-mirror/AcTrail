//! eBPF-backed collector adapter.

pub mod capability_probe;
#[path = "collector/dynamic_go_tls.rs"]
mod collector_dynamic_go_tls;
#[path = "collector/dynamic_tls.rs"]
mod collector_dynamic_tls;
#[path = "collector/events.rs"]
mod collector_events;
#[path = "collector/instance.rs"]
mod collector_instance_impl;
#[path = "collector/net_aggregation.rs"]
mod collector_net_aggregation;
#[path = "collector/runtime.rs"]
mod collector_runtime;
#[path = "collector/stdio_payload.rs"]
mod collector_stdio_payload;
#[path = "collector/trace_binding.rs"]
mod collector_trace_binding;
pub mod decode;
pub mod loader;
pub mod maps;
pub mod procfs;
pub mod sensors;

use std::collections::BTreeMap;
use std::os::fd::{OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use collector_binding::{
    CoverageGuardHandle, CoverageGuardRequest, TraceBindingHandle, TraceBindingRequest,
};
use collector_event::RawCollectorEvent;
use collector_instance::{CollectorError, CollectorInstance, CollectorPollBatch};
use collector_stats::{CollectorStats, DropCounter};
use config_core::daemon::{EbpfCollectorConfig, FileBulkReadFastPathConfig, PayloadConfig};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{
    InitialSuppressedFd, KernelProcessCoordinates, ProcessIdentity, ProcessObservation,
    ProcessRecord, ProcessSuppressedFd,
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
    ArmedLaunchBinding, AttachPlan, EbpfProgramLoader, EbpfRuntime, LoaderError,
    PendingTlsPayloadOp, ProcessIdentityResolutionRequest, TlsPayloadDiagnostics,
};
pub use crate::loader::{LaunchBindingFailure, LaunchBindingFailureStatus, SocketPayloadFdState};
use crate::maps::BindingStateMap;
use collector_dynamic_go_tls::DynamicGoTlsAttacher;
use collector_dynamic_tls::DynamicTlsAttacher;
use collector_net_aggregation::NetAggregator;
use collector_stdio_payload::StdioPayloadAssembler;

pub struct EbpfCollector {
    probe_result: EbpfProbeResult,
    loader: EbpfProgramLoader,
    bindings: BindingStateMap,
    runtime: Option<EbpfRuntime>,
    file_tracker: FileTracker,
    dynamic_go_tls: DynamicGoTlsAttacher,
    dynamic_tls: DynamicTlsAttacher,
    file_bulk_read_fast_path: FileBulkReadFastPathConfig,
    tls_capture_requests: Vec<TlsPayloadCaptureRequest>,
    tls_completions: Vec<TlsPayloadCompletion>,
    tls_direct_captures: Vec<TlsPayloadDirectCapture>,
    tls_diagnostic_events: Vec<TlsDiagnosticEvent>,
    launch_binding_failures: Vec<LaunchBindingFailure>,
    socket_completions: Vec<SocketPayloadCompletion>,
    stdio_payloads: StdioPayloadAssembler,
    net_aggregator: NetAggregator,
    net_aggregation_enabled: Arc<AtomicBool>,
    net_aggregation_backlog: Vec<RawCollectorEvent>,
    suppressed_fds: Vec<TraceSuppressedFd>,
    pending_launches: BTreeMap<TraceId, PendingLaunchBinding>,
    binding_gap_drops: u64,
    binding_gap_lifecycle_skips: u64,
    clock_ticks_per_second: Option<u64>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EbpfPreflightKey(Vec<(Capability, RequestMode)>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct TraceSuppressedFd {
    trace_id: TraceId,
    fd: ProcessSuppressedFd,
}

struct PendingLaunchBinding {
    root_identity: ProcessIdentity,
    root_observation: ProcessObservation,
    generation: u64,
    initial_suppressed_fds: Vec<InitialSuppressedFd>,
    root_working_directory: Option<String>,
    armed_binding: ArmedLaunchBinding,
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
    kernel_tgid: u32,
    start_boottime_ns: u64,
    start_time_ticks: u64,
}

impl KernelForkTraceBinding {
    fn from_runtime(
        kernel_tgid: u32,
        binding: crate::loader::ForkTraceBinding,
        clock_ticks_per_second: u64,
    ) -> Result<Self, CollectorError> {
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
        Ok(Self {
            trace_id: binding.trace_id,
            kernel_tgid,
            start_boottime_ns: binding.child_start_boottime_ns,
            start_time_ticks,
        })
    }

    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub fn kernel_tgid(&self) -> u32 {
        self.kernel_tgid
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
        if host.start_time_ticks != self.start_time_ticks {
            return Err(CollectorError::new(
                "fork_trace_identity",
                format!(
                    "kernel fork binding generation mismatch for kernel TGID {}",
                    self.kernel_tgid
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
        let mcp_stdio_enabled = payload_config.mcp.enabled
            && payload_config.stdio.enabled
            && payload_config.stdio.capture_stdin;
        let file_tracker = FileTracker::new(config.ipc_lineage, mcp_stdio_enabled);
        let probe_result = probe_result_for_config(probe_result, &config, &payload_config);
        let net_aggregation_enabled = Arc::new(AtomicBool::new(config.net_send_recv_aggregation));
        Self {
            probe_result,
            loader: EbpfProgramLoader::new(
                config,
                payload_config.clone(),
                file_bulk_read_fast_path.clone(),
            ),
            bindings: BindingStateMap::default(),
            runtime: None,
            file_tracker,
            dynamic_go_tls: DynamicGoTlsAttacher::new(&payload_config.tls),
            dynamic_tls: DynamicTlsAttacher::default(),
            file_bulk_read_fast_path,
            tls_capture_requests: Vec::new(),
            tls_completions: Vec::new(),
            tls_direct_captures: Vec::new(),
            tls_diagnostic_events: Vec::new(),
            launch_binding_failures: Vec::new(),
            socket_completions: Vec::new(),
            stdio_payloads: StdioPayloadAssembler::new(
                payload_config.stdio.pending_operation_max_entries,
            ),
            net_aggregator: NetAggregator::new(net_aggregation_enabled.clone()),
            net_aggregation_enabled,
            net_aggregation_backlog: Vec::new(),
            suppressed_fds: Vec::new(),
            pending_launches: BTreeMap::new(),
            binding_gap_drops: 0,
            binding_gap_lifecycle_skips: 0,
            clock_ticks_per_second: clock_ticks_per_second(),
        }
    }

    /// Toggle net send/recv aggregation at runtime. Aggregation is additive
    /// (already-buffered state is flushed normally via timeout), so toggling
    /// never drops or duplicates a byte.
    pub fn set_net_send_recv_aggregation(&self, enabled: bool) {
        self.net_aggregation_enabled
            .store(enabled, Ordering::Relaxed);
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
    config: &EbpfCollectorConfig,
    payload: &PayloadConfig,
) -> EbpfProbeResult {
    if !config.ipc_lineage.enabled {
        result.descriptor.capabilities.retain(|descriptor| {
            !matches!(
                &descriptor.capability,
                Capability::IpcPipeFifo | Capability::IpcUnixSocket
            )
        });
    }
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

fn supported_required_capability(
    capability: &Capability,
    config: &EbpfCollectorConfig,
    payload: &PayloadConfig,
) -> bool {
    matches!(
        capability,
        Capability::ProcLifecycle
            | Capability::NetTransport
            | Capability::FsAccessBasic
            | Capability::FsMmap
    ) || (matches!(
        capability,
        Capability::IpcPipeFifo | Capability::IpcUnixSocket
    ) && config.ipc_lineage.enabled)
        || (matches!(capability, Capability::TlsPlaintextPayload)
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

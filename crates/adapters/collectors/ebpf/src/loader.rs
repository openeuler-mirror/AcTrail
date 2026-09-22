//! Program load, attach, and ring-buffer ownership for the eBPF collector.

#[path = "loader/abi/const.rs"]
mod abi;
#[path = "loader/attach_plan/implementation.rs"]
mod attach_plan;
#[path = "loader/consumer.rs"]
mod consumer;
#[path = "loader/program/environment.rs"]
mod environment;
#[path = "loader/fd.rs"]
mod fd;
#[path = "loader/file.rs"]
mod file;
#[path = "loader/file_io/summary.rs"]
mod file_io;
pub(crate) use file_io::{FileIoKey, FileIoSnapshot};
#[path = "loader/launch_binding.rs"]
mod launch_binding;
#[path = "loader/program/object.rs"]
mod object;
#[path = "loader/process.rs"]
mod process;
#[path = "loader/ring_decode.rs"]
mod ring_decode;
#[path = "loader/runtime/implementation.rs"]
mod runtime_implementation;
#[path = "loader/runtime/initialization.rs"]
mod runtime_initialization;
#[path = "loader/runtime/loss.rs"]
mod runtime_loss;
#[path = "loader/runtime/observation_depth.rs"]
mod runtime_observation_depth;
#[path = "loader/runtime/process_identity.rs"]
mod runtime_process_identity;
#[path = "loader/socket.rs"]
mod socket;
#[path = "loader/stdio.rs"]
mod stdio;
#[path = "loader/suppressed_fd.rs"]
mod suppressed_fd;
#[path = "loader/tls.rs"]
mod tls;
#[path = "loader/program/tracepoint.rs"]
mod tracepoint;

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::io::Read;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use config_core::daemon::{
    EbpfCollectorConfig, FileIoSummaryConfig, PayloadConfig, ProcessSeccompConfig,
};
use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder};
use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::{InitialSuppressedFd, KernelProcessCoordinates, ProcessSuppressedFd};

pub use attach_plan::AttachPlan;
use attach_plan::{configure_program_autoload, effective_config_for_attach_plan};
use consumer::{EventConsumer, EventConsumerMessage};
pub(crate) use launch_binding::ArmedLaunchBinding;
use launch_binding::{LaunchBindingTarget, LaunchExecBindings, PendingLaunchBinding};
use object::{event_map_max_entries, map_handle, resize_map, ring_buffer_max_bytes};
use ring_decode::decode_kernel_event;
pub use ring_decode::{
    KernelEndpoint, KernelEndpointRole, KernelEndpointWithRole, KernelEvent, KernelEventIdentity,
    KernelExecPayload, KernelExitPayload, KernelFdIoOperation, KernelFdIoPayload,
    KernelFilePathEvent, KernelForkPayload, KernelNetworkOperation, KernelNetworkPayload,
    KernelObservationCommon, KernelObservationEvent, KernelObservationPayload,
    KernelProcessExecArgEvent, KernelProcessExecAttemptEvent, KernelProcessExecResultEvent,
    KernelProcessForkAttemptEvent, KernelProcessForkResultEvent, KernelSignalPayload,
    KernelSocketFdReleasePayload, KernelSocketPayloadCompletionEvent, KernelSocketPayloadEvent,
    KernelStdioPayloadCompletionEvent, KernelStdioPayloadEvent, KernelTlsCaptureRequestEvent,
    KernelTlsCompletionEvent, KernelTlsDiagnosticEvent, KernelTlsDirectCaptureEvent,
    KernelTlsMappingEvent, LaunchBindingFailure, LaunchBindingFailureStatus,
};
pub use socket::SocketPayloadFdState;
use tls::GoTlsAttachOutcome;
pub use tls::{
    DynamicTlsProbePlan, PendingTlsPayloadOp, TlsPayloadDiagnosticCounter, TlsPayloadDiagnostics,
};

const PID_NAMESPACE_FIELD_SIZE: usize = std::mem::size_of::<u64>();
const PID_NAMESPACE_VALUE_SIZE: usize = PID_NAMESPACE_FIELD_SIZE * 2;
const OBSERVER_PID_NAMESPACE_VALUE_SIZE: usize = PID_NAMESPACE_VALUE_SIZE + 8;
const PROCESS_IDENTITY_VALUE_SIZE: usize = 16;
const PROCESS_IDENTITY_RESOLUTION_KEY_SIZE: usize = 16;
const PROCESS_IDENTITY_RESOLUTION_VALUE_SIZE: usize = 16;
const TRACE_NAMESPACE_THREAD_IDENTITY_VALUE_SIZE: usize = 24;
const LIBBPF_DEBUG_ENV: &str = "ACTRAIL_EBPF_LIBBPF_DEBUG";
const FORK_TRACE_BINDING_TRACE_ID_OFFSET: usize = 0;
const FORK_TRACE_BINDING_CHILD_GENERATION_OFFSET: usize = 16;
const FORK_TRACE_BINDING_VALUE_SIZE: usize = 32;
const OBSERVER_FORK_BINDING_KERNEL_TGID_OFFSET: usize = FORK_TRACE_BINDING_VALUE_SIZE;
const OBSERVER_FORK_BINDING_VALUE_SIZE: usize = FORK_TRACE_BINDING_VALUE_SIZE + 8;
const FORK_IDENTITY_PUBLISH_FAIL_COUNTER: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoaderError {
    pub stage: String,
    pub message: String,
}

impl LoaderError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ForkTraceBinding {
    pub(crate) trace_id: TraceId,
    pub(crate) child_start_boottime_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessIdentityResolutionRequest {
    pub(crate) observer_tgid: u32,
    pub(crate) start_time_ticks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedProcessIdentity {
    pub(crate) kernel_tgid: u32,
    pub(crate) start_boottime_ns: u64,
}

pub struct EbpfProgramLoader {
    config: EbpfCollectorConfig,
    payload: PayloadConfig,
    process: ProcessSeccompConfig,
    file_io_summary_config: FileIoSummaryConfig,
    file_collection: config_core::daemon::FileCollectionConfig,
    file_directory_observation: bool,
    file_tty_observation: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PidNamespace {
    dev: u64,
    ino: u64,
}

pub struct EbpfRuntime {
    pub(crate) attach_plan: AttachPlan,
    /// Dedicated kernel transport consumer. Declared first so it stops (and
    /// drops the kernel buffer) before the BPF object and its map fds close.
    consumer: EventConsumer,
    object: Object,
    links: Vec<Link>,
    attached_programs: Vec<String>,
    attached_capabilities: BTreeSet<Capability>,
    tracked_traces: MapHandle,
    process_observation_depths: MapHandle,
    process_identities: MapHandle,
    process_identity_resolutions: MapHandle,
    trace_namespace_thread_identities: MapHandle,
    observer_pid_diagnostics: MapHandle,
    observer_pid_diagnostics_baseline: ObserverPidDiagnostics,
    launch_bindings: LaunchExecBindings,
    fork_trace_bindings: MapHandle,
    observer_fork_trace_bindings: MapHandle,
    trace_pid_namespaces: MapHandle,
    suppressed_fds: MapHandle,
    suppressed_fd_index: MapHandle,
    suppressed_fd_config: suppressed_fd::SuppressedFdConfig,
    file_io_summaries: Option<file_io::FileIoSummaryMap>,
    pending_tls_payload_ops: MapHandle,
    pending_tls_payload_ops_by_namespace: MapHandle,
    payload_tls_diagnostics: MapHandle,
    tls_diagnostics_baseline: TlsPayloadDiagnostics,
    payload_socket_fds: MapHandle,
    event_transport_diagnostics: MapHandle,
    event_transport_diagnostics_baseline: EventTransportDiagnostics,
    pending_raw_events: Vec<Vec<u8>>,
    last_perf_lost: u64,
    loss_diagnostics: runtime_loss::LossDiagnostics,
    last_raw_sample_count: usize,
}

impl EbpfProgramLoader {
    pub fn new(
        config: EbpfCollectorConfig,
        payload: PayloadConfig,
        process: ProcessSeccompConfig,
        file_io_summary_config: FileIoSummaryConfig,
        file_collection: config_core::daemon::FileCollectionConfig,
        file_directory_observation: bool,
        file_tty_observation: bool,
    ) -> Self {
        Self {
            config,
            payload,
            process,
            file_io_summary_config,
            file_collection,
            file_directory_observation,
            file_tty_observation,
        }
    }

    pub fn config(&self) -> &EbpfCollectorConfig {
        &self.config
    }

    pub(crate) fn file_collection(&self) -> &config_core::daemon::FileCollectionConfig {
        &self.file_collection
    }

    pub(crate) fn file_directory_observation(&self) -> bool {
        self.file_directory_observation
    }

    pub(crate) fn file_tty_observation(&self) -> bool {
        self.file_tty_observation
    }

    pub fn payload_config(&self) -> &PayloadConfig {
        &self.payload
    }
}

fn libbpf_debug_enabled() -> Result<bool, LoaderError> {
    let Some(value) = env::var_os(LIBBPF_DEBUG_ENV) else {
        return Ok(false);
    };
    let value = value.to_string_lossy().to_ascii_lowercase();
    match value.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(LoaderError::new(
            "libbpf_debug_config",
            format!("{LIBBPF_DEBUG_ENV} must be one of 1,true,yes,on,0,false,no,off; got {value}"),
        )),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EventTransportDiagnostics {
    reserve_fail: u64,
    output_fail: u64,
    output_fail_bytes: u64,
    stdio_pending_update_fail: u64,
    stdio_read_user_fail: u64,
    socket_state_update_fail: u64,
    socket_sequence_update_fail: u64,
    process_identity_cache_miss: u64,
    process_identity_cleanup_fail: u64,
    socket_read_user_fail: u64,
    socket_reserve_fail: u64,
    file_pending_update_fail: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ObserverPidDiagnostics {
    level_discovery: u64,
    level_mismatch: u64,
    resolution_fail: u64,
    index_publish_fail: u64,
}

impl ObserverPidDiagnostics {
    fn saturating_delta_since(self, baseline: Self) -> Self {
        Self {
            level_discovery: self
                .level_discovery
                .saturating_sub(baseline.level_discovery),
            level_mismatch: self.level_mismatch.saturating_sub(baseline.level_mismatch),
            resolution_fail: self
                .resolution_fail
                .saturating_sub(baseline.resolution_fail),
            index_publish_fail: self
                .index_publish_fail
                .saturating_sub(baseline.index_publish_fail),
        }
    }
}

impl EventTransportDiagnostics {
    fn saturating_delta_since(self, baseline: Self) -> Self {
        Self {
            reserve_fail: self.reserve_fail.saturating_sub(baseline.reserve_fail),
            output_fail: self.output_fail.saturating_sub(baseline.output_fail),
            output_fail_bytes: self
                .output_fail_bytes
                .saturating_sub(baseline.output_fail_bytes),
            stdio_pending_update_fail: self
                .stdio_pending_update_fail
                .saturating_sub(baseline.stdio_pending_update_fail),
            stdio_read_user_fail: self
                .stdio_read_user_fail
                .saturating_sub(baseline.stdio_read_user_fail),
            socket_state_update_fail: self
                .socket_state_update_fail
                .saturating_sub(baseline.socket_state_update_fail),
            socket_sequence_update_fail: self
                .socket_sequence_update_fail
                .saturating_sub(baseline.socket_sequence_update_fail),
            process_identity_cache_miss: self
                .process_identity_cache_miss
                .saturating_sub(baseline.process_identity_cache_miss),
            process_identity_cleanup_fail: self
                .process_identity_cleanup_fail
                .saturating_sub(baseline.process_identity_cleanup_fail),
            socket_read_user_fail: self
                .socket_read_user_fail
                .saturating_sub(baseline.socket_read_user_fail),
            socket_reserve_fail: self
                .socket_reserve_fail
                .saturating_sub(baseline.socket_reserve_fail),
            file_pending_update_fail: self
                .file_pending_update_fail
                .saturating_sub(baseline.file_pending_update_fail),
        }
    }
}

fn read_event_transport_diagnostics(
    map: &MapHandle,
) -> Result<EventTransportDiagnostics, LoaderError> {
    // The diagnostics map is a fixed-size ARRAY of counters; a single batch
    // lookup returns all entries in one syscall instead of separate lookups.
    // This runs twice per drain cycle, so the saving is material.
    let mut diagnostics = EventTransportDiagnostics::default();
    let mut seen = [false; 13];
    let batch = map
        .lookup_batch(13, MapFlags::ANY, MapFlags::ANY)
        .map_err(|error| LoaderError::new("event_transport_diagnostics", error.to_string()))?;
    for item in batch {
        let (key, value) = item;
        let counter_id = key
            .get(..4)
            .and_then(|raw| raw.try_into().ok())
            .map(u32::from_ne_bytes)
            .ok_or_else(|| {
                LoaderError::new(
                    "event_transport_diagnostics",
                    format!("unexpected counter key size {}", key.len()),
                )
            })?;
        let count = value
            .get(..8)
            .and_then(|raw| raw.try_into().ok())
            .map(u64::from_ne_bytes)
            .ok_or_else(|| {
                LoaderError::new(
                    "event_transport_diagnostics",
                    format!("unexpected counter size {}", value.len()),
                )
            })?;
        if let Some(slot) = seen.get_mut(counter_id as usize) {
            *slot = true;
        }
        match counter_id {
            0 => diagnostics.reserve_fail = count,
            1 => diagnostics.output_fail = count,
            2 => diagnostics.output_fail_bytes = count,
            4 => diagnostics.stdio_pending_update_fail = count,
            5 => diagnostics.stdio_read_user_fail = count,
            6 => diagnostics.socket_state_update_fail = count,
            7 => diagnostics.socket_sequence_update_fail = count,
            8 => diagnostics.process_identity_cache_miss = count,
            9 => diagnostics.process_identity_cleanup_fail = count,
            10 => diagnostics.socket_read_user_fail = count,
            11 => diagnostics.socket_reserve_fail = count,
            12 => diagnostics.file_pending_update_fail = count,
            _ => {}
        }
    }
    for counter_id in [0_u32, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
        if !seen[counter_id as usize] {
            return Err(LoaderError::new(
                "event_transport_diagnostics",
                format!("missing counter {counter_id}"),
            ));
        }
    }
    Ok(diagnostics)
}

fn read_observer_pid_diagnostics(map: &MapHandle) -> Result<ObserverPidDiagnostics, LoaderError> {
    let mut diagnostics = ObserverPidDiagnostics::default();
    let mut seen = [false; 5];
    let batch = map
        .lookup_batch(5, MapFlags::ANY, MapFlags::ANY)
        .map_err(|error| LoaderError::new("observer_pid_diagnostics", error.to_string()))?;
    for (key, value) in batch {
        let counter_id = key
            .get(..4)
            .and_then(|raw| raw.try_into().ok())
            .map(u32::from_ne_bytes)
            .ok_or_else(|| {
                LoaderError::new(
                    "observer_pid_diagnostics",
                    format!("unexpected counter key size {}", key.len()),
                )
            })?;
        let count = value.chunks_exact(8).try_fold(0_u64, |total, raw| {
            let counter = u64::from_ne_bytes(raw.try_into().map_err(|_| {
                LoaderError::new("observer_pid_diagnostics", "truncated per-CPU counter")
            })?);
            Ok::<u64, LoaderError>(total.saturating_add(counter))
        })?;
        if let Some(slot) = seen.get_mut(counter_id as usize) {
            *slot = true;
        }
        match counter_id {
            1 => diagnostics.level_discovery = count,
            2 => diagnostics.level_mismatch = count,
            3 => diagnostics.resolution_fail = count,
            4 => diagnostics.index_publish_fail = count,
            _ => {}
        }
    }
    for counter_id in 0_u32..5 {
        if !seen[counter_id as usize] {
            return Err(LoaderError::new(
                "observer_pid_diagnostics",
                format!("missing counter {counter_id}"),
            ));
        }
    }
    Ok(diagnostics)
}

fn read_event_transport_counter(map: &MapHandle, counter_id: u32) -> Result<u64, LoaderError> {
    map.lookup(&counter_id.to_ne_bytes(), MapFlags::ANY)
        .map_err(|error| LoaderError::new("event_transport_diagnostics", error.to_string()))?
        .map(|value| {
            value
                .get(..8)
                .and_then(|raw| raw.try_into().ok())
                .map(u64::from_ne_bytes)
                .ok_or_else(|| {
                    LoaderError::new(
                        "event_transport_diagnostics",
                        format!("unexpected counter size {}", value.len()),
                    )
                })
        })
        .transpose()?
        .ok_or_else(|| {
            LoaderError::new(
                "event_transport_diagnostics",
                format!("missing counter {counter_id}"),
            )
        })
}

fn parse_fork_trace_binding(value: &[u8]) -> Result<ForkTraceBinding, LoaderError> {
    if value.len() != FORK_TRACE_BINDING_VALUE_SIZE {
        return Err(LoaderError::new(
            "fork_trace_binding",
            format!("unexpected fork trace binding size {}", value.len()),
        ));
    }
    let trace_id = value[FORK_TRACE_BINDING_TRACE_ID_OFFSET..][..8]
        .try_into()
        .map(u64::from_ne_bytes)
        .map(TraceId::new)
        .map_err(|_| LoaderError::new("fork_trace_binding", "invalid trace id field"))?;
    let child_start_boottime_ns = value[FORK_TRACE_BINDING_CHILD_GENERATION_OFFSET..][..8]
        .try_into()
        .map(u64::from_ne_bytes)
        .map_err(|_| LoaderError::new("fork_trace_binding", "invalid child generation field"))?;
    if trace_id.get() == 0 || child_start_boottime_ns == 0 {
        return Err(LoaderError::new(
            "fork_trace_binding",
            "fork trace binding contains an empty identity",
        ));
    }
    Ok(ForkTraceBinding {
        trace_id,
        child_start_boottime_ns,
    })
}

fn read_pid_namespace_for_pid(pid: u32) -> Result<PidNamespace, LoaderError> {
    let path = format!("/proc/{pid}/ns/pid");
    let metadata = std::fs::metadata(&path)
        .map_err(|error| LoaderError::new("trace_pid_namespace", error.to_string()))?;
    Ok(PidNamespace {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

fn read_observer_pid_namespace() -> Result<PidNamespace, LoaderError> {
    let observer = read_pid_namespace_for_pid(std::process::id())?;
    let procfs_root = read_pid_namespace_for_pid(1)?;
    if observer != procfs_root {
        return Err(LoaderError::new(
            "observer_pid_namespace",
            format!(
                "actraild PID namespace dev={} ino={} does not match mounted procfs PID namespace dev={} ino={}",
                observer.dev, observer.ino, procfs_root.dev, procfs_root.ino
            ),
        ));
    }
    Ok(observer)
}

fn write_trace_pid_namespace(
    trace_pid_namespaces: &MapHandle,
    trace_id: TraceId,
    namespace: PidNamespace,
    stage: &'static str,
) -> Result<(), LoaderError> {
    let key = trace_id.get().to_ne_bytes();
    let mut value = [0_u8; PID_NAMESPACE_VALUE_SIZE];
    value[0..PID_NAMESPACE_FIELD_SIZE].copy_from_slice(&namespace.dev.to_ne_bytes());
    value[PID_NAMESPACE_FIELD_SIZE..PID_NAMESPACE_VALUE_SIZE]
        .copy_from_slice(&namespace.ino.to_ne_bytes());
    trace_pid_namespaces
        .update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new(stage, error.to_string()))
}

fn write_observer_pid_namespace(
    map: &MapHandle,
    namespace: PidNamespace,
) -> Result<(), LoaderError> {
    let key = 0_u32.to_ne_bytes();
    let mut value = [0_u8; OBSERVER_PID_NAMESPACE_VALUE_SIZE];
    value[0..PID_NAMESPACE_FIELD_SIZE].copy_from_slice(&namespace.dev.to_ne_bytes());
    value[PID_NAMESPACE_FIELD_SIZE..PID_NAMESPACE_VALUE_SIZE]
        .copy_from_slice(&namespace.ino.to_ne_bytes());
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("observer_pid_namespace", error.to_string()))
}

fn parse_observer_fork_trace_binding(raw: &[u8]) -> Result<(u32, ForkTraceBinding), LoaderError> {
    if raw.len() != OBSERVER_FORK_BINDING_VALUE_SIZE {
        return Err(LoaderError::new(
            "fork_trace_binding",
            format!("unexpected observer fork binding size {}", raw.len()),
        ));
    }
    let kernel_tgid = u32::from_ne_bytes(
        raw[OBSERVER_FORK_BINDING_KERNEL_TGID_OFFSET..OBSERVER_FORK_BINDING_KERNEL_TGID_OFFSET + 4]
            .try_into()
            .expect("observer fork binding kernel TGID slice has fixed size"),
    );
    Ok((
        kernel_tgid,
        parse_fork_trace_binding(&raw[..FORK_TRACE_BINDING_VALUE_SIZE])?,
    ))
}

//! Program load, attach, and ring-buffer ownership for the eBPF collector.

#[path = "loader/abi/const.rs"]
mod abi;
#[path = "loader/attach_plan.rs"]
mod attach_plan;
#[path = "loader/program/environment.rs"]
mod environment;
#[path = "loader/file.rs"]
mod file;
#[path = "loader/launch_binding.rs"]
mod launch_binding;
#[path = "loader/program/object.rs"]
mod object;
#[path = "loader/ring_decode.rs"]
mod ring_decode;
#[path = "loader/runtime/implementation.rs"]
mod runtime_implementation;
#[path = "loader/runtime/link_teardown.rs"]
mod runtime_link_teardown;
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

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;

use config_core::daemon::{EbpfCollectorConfig, FileBulkReadFastPathConfig, PayloadConfig};
use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder};
use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::{InitialSuppressedFd, KernelProcessCoordinates, ProcessSuppressedFd};

pub use attach_plan::AttachPlan;
use attach_plan::{configure_program_autoload, effective_config_for_attach_plan};
pub(crate) use launch_binding::ArmedLaunchBinding;
use launch_binding::{LaunchBindingTarget, LaunchExecBindings, PendingLaunchBinding};
use object::{EventBuffer, event_map_max_entries, map_handle, resize_map, ring_buffer_max_bytes};
pub use ring_decode::{
    KernelEndpoint, KernelEvent, KernelFilePathEvent, KernelObservationEvent,
    KernelSocketPayloadCompletionEvent, KernelSocketPayloadEvent,
    KernelStdioPayloadCompletionEvent, KernelStdioPayloadEvent, KernelTlsCaptureRequestEvent,
    KernelTlsCompletionEvent, KernelTlsDiagnosticEvent, KernelTlsDirectCaptureEvent,
    LaunchBindingFailure, LaunchBindingFailureStatus,
};
use runtime_link_teardown::StaticLinkTeardown;
pub use socket::SocketPayloadFdState;
use tls::GoTlsAttachOutcome;
pub use tls::{
    DynamicTlsProbePlan, PendingTlsPayloadOp, TlsPayloadDiagnosticCounter, TlsPayloadDiagnostics,
};

const PID_NAMESPACE_FIELD_SIZE: usize = std::mem::size_of::<u64>();
const PID_NAMESPACE_VALUE_SIZE: usize = PID_NAMESPACE_FIELD_SIZE * 2;
const FILE_BULK_READ_FAST_PROCESS_KEY_SIZE: usize =
    std::mem::size_of::<u32>() + std::mem::size_of::<u64>();
const FILE_BULK_READ_FAST_PROCESS_VALUE_SIZE: usize = std::mem::size_of::<u64>();
const FILE_BULK_READ_FAST_FD_KEY_SIZE: usize =
    std::mem::size_of::<u32>() + std::mem::size_of::<u32>() + std::mem::size_of::<u64>();
const LIBBPF_DEBUG_ENV: &str = "ACTRAIL_EBPF_LIBBPF_DEBUG";
const FORK_TRACE_BINDING_TRACE_ID_OFFSET: usize = 0;
const FORK_TRACE_BINDING_CHILD_GENERATION_OFFSET: usize = 16;
const FORK_TRACE_BINDING_VALUE_SIZE: usize = 32;
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

pub struct EbpfProgramLoader {
    config: EbpfCollectorConfig,
    payload: PayloadConfig,
    file_bulk_read_fast_path: FileBulkReadFastPathConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PidNamespace {
    dev: u64,
    ino: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileBulkReadFastFdKey {
    pid: u32,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeAttachmentState {
    Parked,
    Attached,
}

pub struct EbpfRuntime {
    object: Object,
    links: Vec<Link>,
    static_link_teardown: StaticLinkTeardown,
    attachment_state: RuntimeAttachmentState,
    attach_plan: AttachPlan,
    payload: PayloadConfig,
    planned_static_programs: Vec<String>,
    planned_capabilities: BTreeSet<Capability>,
    attached_programs: Vec<String>,
    attached_capabilities: BTreeSet<Capability>,
    tracked_traces: MapHandle,
    process_start_times: MapHandle,
    launch_bindings: LaunchExecBindings,
    fork_trace_bindings: MapHandle,
    trace_pid_namespaces: MapHandle,
    suppressed_fds: MapHandle,
    suppressed_fd_index: MapHandle,
    suppressed_fd_index_slots_per_process: u32,
    file_bulk_read_fast_processes: MapHandle,
    file_bulk_read_fast_fd_stats: MapHandle,
    pending_tls_payload_ops: MapHandle,
    pending_tls_payload_ops_by_namespace: MapHandle,
    payload_tls_diagnostics: MapHandle,
    tls_diagnostics_baseline: TlsPayloadDiagnostics,
    payload_socket_fds: MapHandle,
    event_transport_diagnostics: MapHandle,
    event_transport_diagnostics_baseline: EventTransportDiagnostics,
    events_map: MapHandle,
    events: Rc<RefCell<Vec<KernelEvent>>>,
    decode_error: Rc<RefCell<Option<LoaderError>>>,
    event_buffer_bytes: u32,
    event_buffer: Option<EventBuffer>,
    last_event_transport_loss_summary: Option<String>,
    pending_event_transport_loss_summaries: Vec<String>,
    last_raw_sample_count: usize,
}

impl EbpfProgramLoader {
    pub fn new(
        config: EbpfCollectorConfig,
        payload: PayloadConfig,
        file_bulk_read_fast_path: FileBulkReadFastPathConfig,
    ) -> Self {
        Self {
            config,
            payload,
            file_bulk_read_fast_path,
        }
    }

    pub fn config(&self) -> &EbpfCollectorConfig {
        &self.config
    }

    pub fn payload_config(&self) -> &PayloadConfig {
        &self.payload
    }

    pub fn load_runtime_with_plan(
        &self,
        attach_plan: &AttachPlan,
    ) -> Result<EbpfRuntime, LoaderError> {
        let static_link_teardown =
            StaticLinkTeardown::new(self.config.preflight_link_teardown_workers)?;
        file::validate_file_config(&self.config)?;
        tls::validate_payload_config(&self.payload.tls)?;
        stdio::validate_payload_config(&self.payload.stdio)?;
        socket::validate_payload_config(&self.payload.socket)?;
        suppressed_fd::validate_config(&self.config)?;
        let effective_payload = effective_config_for_attach_plan(&self.payload, attach_plan);
        environment::ensure_tracefs_control()?;
        environment::apply_memlock_rlimit(self.config.memlock_rlimit)?;
        let object_bytes = include_bytes!(env!("ACTRAIL_EBPF_OBJECT"));
        let mut builder = ObjectBuilder::default();
        if libbpf_debug_enabled()? {
            builder.debug(true);
        }
        let mut open_object = builder
            .open_memory(object_bytes)
            .map_err(|error| LoaderError::new("open_object", error.to_string()))?;
        resize_map(
            &mut open_object,
            "tracked_traces",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "process_start_times",
            self.config.tracked_process_max_entries,
        )?;
        #[cfg(actrail_launch_binding_pid_generation_hash)]
        {
            resize_map(
                &mut open_object,
                "pending_exec_bindings",
                self.config.pending_operation_max_entries,
            )?;
            resize_map(
                &mut open_object,
                "pending_exec_pid_index",
                self.config.pending_operation_max_entries,
            )?;
        }
        resize_map(
            &mut open_object,
            "trace_pid_namespaces",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_net_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_ipc_fd_pair_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "fork_trace_bindings",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_exit_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "suppressed_fds",
            self.config.suppressed_fd_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "suppressed_fd_index",
            self.config.suppressed_fd_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_suppressed_fd_dup_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "file_bulk_read_fast_processes",
            self.file_bulk_read_fast_path.process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "file_bulk_read_fast_fd_stats",
            self.file_bulk_read_fast_path.fd_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_file_bulk_read_fast_ops",
            self.file_bulk_read_fast_path.pending_op_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_tls_payload_ops",
            effective_payload.tls.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "tls_pending_ns",
            effective_payload.tls.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "go_tls_read_buffers",
            effective_payload.tls.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_stdio_payload_ops",
            effective_payload.stdio.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_stdio_stream_sequences",
            effective_payload.stdio.stream_state_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_fds",
            effective_payload.socket.stream_state_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_process_generations",
            effective_payload.socket.stream_state_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_socket_payload_ops",
            effective_payload.socket.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_socket_dup_ops",
            effective_payload.socket.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_stream_sequences",
            effective_payload.socket.stream_state_max_entries,
        )?;
        let event_buffer_bytes = ring_buffer_max_bytes(&self.config, &effective_payload);
        resize_map(
            &mut open_object,
            "events",
            event_map_max_entries(event_buffer_bytes)?,
        )?;
        configure_program_autoload(&mut open_object, attach_plan)?;

        let object = open_object
            .load()
            .map_err(|error| LoaderError::new("load_object", error.to_string()))?;
        EbpfRuntime::from_object(
            object,
            &self.config,
            &effective_payload,
            attach_plan,
            static_link_teardown,
        )
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
        }
    }
}

fn read_event_transport_diagnostics(
    map: &MapHandle,
) -> Result<EventTransportDiagnostics, LoaderError> {
    // The diagnostics map is a fixed-size ARRAY of counters; a single batch
    // lookup returns all entries in one syscall instead of five lookups.
    // This runs twice per drain cycle, so the saving is material.
    let mut diagnostics = EventTransportDiagnostics::default();
    let mut seen = [false; 6];
    let batch = map
        .lookup_batch(6, MapFlags::ANY, MapFlags::ANY)
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
            _ => {}
        }
    }
    for counter_id in [0_u32, 1, 2, 4, 5] {
        if !seen[counter_id as usize] {
            return Err(LoaderError::new(
                "event_transport_diagnostics",
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

fn file_bulk_read_fast_process_key(
    pid: u32,
    generation: u64,
) -> Result<[u8; FILE_BULK_READ_FAST_PROCESS_KEY_SIZE], LoaderError> {
    if generation == 0 {
        return Err(LoaderError::new(
            "file_bulk_read_fast_process",
            "fast path process key requires a non-zero process generation",
        ));
    }
    let mut key = [0_u8; FILE_BULK_READ_FAST_PROCESS_KEY_SIZE];
    key[0..4].copy_from_slice(&pid.to_ne_bytes());
    key[4..12].copy_from_slice(&generation.to_ne_bytes());
    Ok(key)
}

fn parse_file_bulk_read_fast_fd_key(raw: &[u8]) -> Option<FileBulkReadFastFdKey> {
    if raw.len() != FILE_BULK_READ_FAST_FD_KEY_SIZE {
        return None;
    }
    Some(FileBulkReadFastFdKey {
        pid: u32::from_ne_bytes(raw[0..4].try_into().ok()?),
        generation: u64::from_ne_bytes(raw[8..16].try_into().ok()?),
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

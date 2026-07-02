//! Program load, attach, and ring-buffer ownership for the eBPF collector.

#[path = "loader/abi/const.rs"]
mod abi;
#[path = "loader/attach_plan.rs"]
mod attach_plan;
#[path = "loader/environment.rs"]
mod environment;
#[path = "loader/file.rs"]
mod file;
#[path = "loader/fork.rs"]
mod fork;
#[path = "loader/object.rs"]
mod object;
#[path = "loader/ring_decode.rs"]
mod ring_decode;
#[path = "loader/socket.rs"]
mod socket;
#[path = "loader/stdio.rs"]
mod stdio;
#[path = "loader/suppressed_fd.rs"]
mod suppressed_fd;
#[path = "loader/tls.rs"]
mod tls;
#[path = "loader/tracepoint.rs"]
mod tracepoint;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::os::fd::RawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;

use config_core::daemon::{EbpfCollectorConfig, FileBulkReadFastPathConfig, PayloadConfig};
use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder};
use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessSuppressedFd};

pub use attach_plan::AttachPlan;
use attach_plan::{configure_program_autoload, effective_config_for_attach_plan};
use object::{EventBuffer, event_map_max_entries, map_handle, resize_map, ring_buffer_max_bytes};
use ring_decode::decode_kernel_event;
pub use ring_decode::{
    KernelEndpoint, KernelEvent, KernelFilePathEvent, KernelObservationEvent,
    KernelSocketPayloadCompletionEvent, KernelSocketPayloadEvent, KernelStdioPayloadEvent,
    KernelTlsCaptureRequestEvent, KernelTlsCompletionEvent, KernelTlsDiagnosticEvent,
    KernelTlsDirectCaptureEvent,
};
use tls::GoTlsAttachOutcome;
pub use tls::{PendingTlsPayloadOp, TlsPayloadDiagnosticCounter, TlsPayloadDiagnostics};

const ACTIVE_PID_NAMESPACE_KEY: u32 = 0;
const PID_NAMESPACE_FIELD_SIZE: usize = std::mem::size_of::<u64>();
const PID_NAMESPACE_VALUE_SIZE: usize = PID_NAMESPACE_FIELD_SIZE * 2;
const FILE_BULK_READ_FAST_PROCESS_KEY_SIZE: usize =
    std::mem::size_of::<u32>() + std::mem::size_of::<u64>();
const FILE_BULK_READ_FAST_PROCESS_VALUE_SIZE: usize = std::mem::size_of::<u64>();
const FILE_BULK_READ_FAST_FD_KEY_SIZE: usize =
    std::mem::size_of::<u32>() + std::mem::size_of::<u32>() + std::mem::size_of::<u64>();
const LIBBPF_DEBUG_ENV: &str = "ACTRAIL_EBPF_LIBBPF_DEBUG";

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

pub struct EbpfRuntime {
    _object: Object,
    _links: Vec<Link>,
    attached_programs: Vec<String>,
    attached_capabilities: BTreeSet<Capability>,
    tracked_traces: MapHandle,
    process_generations: MapHandle,
    pid_namespace: MapHandle,
    suppressed_fds: MapHandle,
    suppressed_fd_index: MapHandle,
    suppressed_fd_index_slots_per_process: u32,
    file_bulk_read_fast_processes: MapHandle,
    file_bulk_read_fast_fd_stats: MapHandle,
    pending_tls_payload_ops: MapHandle,
    pending_tls_payload_ops_by_namespace: MapHandle,
    payload_tls_diagnostics: MapHandle,
    payload_socket_fds: MapHandle,
    event_transport_diagnostics: MapHandle,
    events: Rc<RefCell<Vec<Vec<u8>>>>,
    event_buffer: EventBuffer,
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
            "process_generations",
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
            "pending_child_proc_ops",
            self.config.pending_operation_max_entries,
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
        EbpfRuntime::from_object(object, &self.config, &effective_payload, attach_plan)
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

impl EbpfRuntime {
    fn from_object(
        mut object: Object,
        config: &EbpfCollectorConfig,
        payload: &PayloadConfig,
        attach_plan: &AttachPlan,
    ) -> Result<Self, LoaderError> {
        let tracked_traces = map_handle(&object, "tracked_traces", "tracked_map")?;
        let process_generations =
            map_handle(&object, "process_generations", "process_generation_map")?;
        let pid_namespace = map_handle(&object, "pid_namespace", "pid_namespace_map")?;
        let suppressed_fds = map_handle(&object, "suppressed_fds", "suppressed_fds")?;
        let suppressed_fd_index =
            map_handle(&object, "suppressed_fd_index", "suppressed_fd_index")?;
        let file_bulk_read_fast_processes = map_handle(
            &object,
            "file_bulk_read_fast_processes",
            "file_bulk_read_fast_processes",
        )?;
        let file_bulk_read_fast_fd_stats = map_handle(
            &object,
            "file_bulk_read_fast_fd_stats",
            "file_bulk_read_fast_fd_stats",
        )?;
        let pending_tls_payload_ops = map_handle(
            &object,
            "pending_tls_payload_ops",
            "pending_tls_payload_ops",
        )?;
        let pending_tls_payload_ops_by_namespace =
            map_handle(&object, "tls_pending_ns", "tls_pending_ns")?;
        let payload_tls_diagnostics = map_handle(
            &object,
            "payload_tls_diagnostics",
            "payload_tls_diagnostics",
        )?;
        let payload_socket_fds = map_handle(&object, "payload_socket_fds", "payload_socket_fds")?;
        let event_transport_diagnostics = map_handle(
            &object,
            "event_transport_diagnostics",
            "event_transport_diagnostics",
        )?;
        let events_map = map_handle(&object, "events", "event_buffer")?;

        let events = Rc::new(RefCell::new(Vec::new()));
        let event_buffer = EventBuffer::build(
            &events_map,
            Rc::clone(&events),
            ring_buffer_max_bytes(config, payload),
        )?;
        configure_collector_pid_namespace(&pid_namespace)?;
        fork::configure_child_pid_offset_map(&object, attach_plan)?;
        file::configure_file_config_map(&object, config)?;
        suppressed_fd::configure_config_map(&object, config)?;
        tls::configure_payload_tls_map(&object, &payload.tls)?;
        stdio::configure_payload_stdio_map(&object, &payload.stdio)?;
        socket::configure_payload_socket_map(&object, &payload.socket)?;

        let mut links = Vec::new();
        let mut attached_programs = Vec::new();
        let mut autoloaded_programs = object
            .progs()
            .filter(|program| program.autoload())
            .map(|program| program.name().to_string_lossy().into_owned())
            .filter(|program_name| !tls::is_payload_tls_program(program_name))
            .collect::<Vec<_>>();
        autoloaded_programs.sort_by_key(|program_name| attach_plan.attach_priority(program_name));
        for program_name in autoloaded_programs {
            let program = object
                .progs_mut()
                .find(|program| program.name() == OsStr::new(&program_name))
                .ok_or_else(|| {
                    LoaderError::new(
                        "attach_program",
                        format!("BPF program {program_name} is missing"),
                    )
                })?;
            if let Some(link) = tracepoint::attach_program(
                &program,
                &program_name,
                attach_plan.allows_missing_tracepoint(&program_name),
            )? {
                links.push(link);
                attached_programs.push(program_name);
            }
        }
        for (link, program_name) in tls::attach_payload_tls_programs(&mut object, &payload.tls)? {
            links.push(link);
            attached_programs.push(program_name);
        }
        if links.is_empty() {
            return Err(LoaderError::new(
                "attach_program",
                "eBPF object did not attach any programs",
            ));
        }

        let attached_capabilities = attach_plan.attached_capabilities(&attached_programs);

        Ok(Self {
            _object: object,
            _links: links,
            attached_programs,
            attached_capabilities,
            tracked_traces,
            process_generations,
            pid_namespace,
            suppressed_fds,
            suppressed_fd_index,
            suppressed_fd_index_slots_per_process: config.suppressed_fd_index_slots_per_process,
            file_bulk_read_fast_processes,
            file_bulk_read_fast_fd_stats,
            pending_tls_payload_ops,
            pending_tls_payload_ops_by_namespace,
            payload_tls_diagnostics,
            payload_socket_fds,
            event_transport_diagnostics,
            events,
            event_buffer,
            last_event_transport_loss_summary: None,
            pending_event_transport_loss_summaries: Vec::new(),
            last_raw_sample_count: 0,
        })
    }

    pub fn poll_events(&mut self) -> Result<Vec<KernelEvent>, LoaderError> {
        self.event_buffer.consume()?;
        self.capture_event_transport_loss()?;
        let raw_events = std::mem::take(&mut *self.events.borrow_mut());
        self.last_raw_sample_count = raw_events.len();
        raw_events
            .into_iter()
            .map(|raw| decode_kernel_event(&raw))
            .collect()
    }

    /// Drain the kernel transport buffer into userspace without decoding.
    ///
    /// Call this after a drain cycle's expensive processing to shrink the
    /// starvation window — events that arrived while the pipeline was busy
    /// are moved into the userspace raw buffer so the kernel ring buffer can
    /// accept new submissions. The buffered bytes are decoded on the next
    /// `poll_events()` call.
    pub fn flush_transport(&mut self) -> Result<(), LoaderError> {
        self.event_buffer.consume()?;
        self.capture_event_transport_loss()?;
        Ok(())
    }

    fn capture_event_transport_loss(&mut self) -> Result<(), LoaderError> {
        let perf_lost = self.event_buffer.lost_count();
        let diagnostics = read_event_transport_diagnostics(&self.event_transport_diagnostics)?;
        if perf_lost != 0
            || diagnostics.reserve_fail != 0
            || diagnostics.output_fail != 0
            || diagnostics.output_fail_bytes != 0
        {
            let summary = format!(
                "kernel event transport lost data: perf_lost={perf_lost}, reserve_fail={}, output_fail={}, output_fail_bytes={}",
                diagnostics.reserve_fail, diagnostics.output_fail, diagnostics.output_fail_bytes
            );
            if self.last_event_transport_loss_summary.as_deref() != Some(summary.as_str()) {
                self.last_event_transport_loss_summary = Some(summary.clone());
                self.pending_event_transport_loss_summaries.push(summary);
            }
        }
        Ok(())
    }

    pub fn take_event_transport_loss_summaries(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_event_transport_loss_summaries)
    }

    pub fn track_pid(
        &self,
        map_pid: u32,
        identity: &ProcessIdentity,
        trace_id: TraceId,
    ) -> Result<(), LoaderError> {
        let key = map_pid.to_ne_bytes();
        let value = trace_id.get().to_ne_bytes();
        self.tracked_traces
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid", error.to_string()))?;
        self.process_generations
            .update(&key, &identity.generation.to_ne_bytes(), MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid_generation", error.to_string()))
    }

    pub fn configure_pid_namespace_for_pid(&self, pid: u32) -> Result<(), LoaderError> {
        let namespace = read_pid_namespace_for_pid(pid)?;
        write_pid_namespace_map(&self.pid_namespace, namespace, "trace_pid_namespace")
    }

    pub fn suppress_fd(
        &self,
        trace_id: TraceId,
        suppressed_fd: &ProcessSuppressedFd,
    ) -> Result<(), LoaderError> {
        suppressed_fd::suppress_fd(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            self.suppressed_fd_index_slots_per_process,
            trace_id,
            suppressed_fd,
        )
    }

    pub fn unsuppress_fd(&self, process: &ProcessIdentity, fd: i32) -> Result<(), LoaderError> {
        suppressed_fd::unsuppress_fd(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            self.suppressed_fd_index_slots_per_process,
            process,
            fd,
        )
    }

    pub fn sweep_suppressed_fds_for_process(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<(), LoaderError> {
        suppressed_fd::sweep_process(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            pid,
            generation,
        )
    }

    pub fn sweep_suppressed_fds_for_trace(&self, trace_id: TraceId) -> Result<(), LoaderError> {
        suppressed_fd::sweep_trace(&self.suppressed_fds, &self.suppressed_fd_index, trace_id)
    }

    pub fn tracked_trace_id(&self, pid: u32) -> Result<Option<TraceId>, LoaderError> {
        let key = pid.to_ne_bytes();
        self.tracked_traces
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("lookup_tracked_pid", error.to_string()))?
            .map(|value| {
                value
                    .get(..8)
                    .and_then(|value| value.try_into().ok())
                    .map(u64::from_ne_bytes)
                    .map(TraceId::new)
                    .ok_or_else(|| {
                        LoaderError::new(
                            "lookup_tracked_pid",
                            format!("unexpected tracked trace value size {}", value.len()),
                        )
                    })
            })
            .transpose()
    }

    pub fn lookup_socket_fd_generation(
        &self,
        pid: u32,
        fd: u32,
    ) -> Result<Option<u32>, LoaderError> {
        socket::lookup_fd_generation(&self.payload_socket_fds, pid, fd)
    }

    pub fn attached_programs(&self) -> &[String] {
        &self.attached_programs
    }

    pub fn attached_capabilities(&self) -> &BTreeSet<Capability> {
        &self.attached_capabilities
    }

    pub fn last_raw_sample_count(&self) -> usize {
        self.last_raw_sample_count
    }

    pub fn untrack_pid(&self, pid: u32) -> Result<(), LoaderError> {
        if self.tracked_trace_id(pid)?.is_none() {
            return Ok(());
        }
        let key = pid.to_ne_bytes();
        self.tracked_traces
            .delete(&key)
            .map_err(|error| LoaderError::new("untrack_pid", error.to_string()))?;
        self.process_generations
            .delete(&key)
            .map_err(|error| LoaderError::new("untrack_pid_generation", error.to_string()))
    }

    pub fn mark_file_bulk_read_fast_process(
        &self,
        pid: u32,
        generation: u64,
        trace_id: TraceId,
    ) -> Result<(), LoaderError> {
        let key = file_bulk_read_fast_process_key(pid, generation)?;
        let mut value = [0_u8; FILE_BULK_READ_FAST_PROCESS_VALUE_SIZE];
        value.copy_from_slice(&trace_id.get().to_ne_bytes());
        self.file_bulk_read_fast_processes
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("file_bulk_read_fast_process", error.to_string()))
    }

    pub fn unmark_file_bulk_read_fast_process(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<(), LoaderError> {
        let key = file_bulk_read_fast_process_key(pid, generation)?;
        if self
            .file_bulk_read_fast_processes
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("file_bulk_read_fast_process", error.to_string()))?
            .is_none()
        {
            return Ok(());
        }
        self.file_bulk_read_fast_processes
            .delete(&key)
            .map_err(|error| LoaderError::new("file_bulk_read_fast_process", error.to_string()))
    }

    pub fn sweep_file_bulk_read_fast_fds_for_process(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<(), LoaderError> {
        for key in self.file_bulk_read_fast_fd_stats.keys().collect::<Vec<_>>() {
            let Some(parsed) = parse_file_bulk_read_fast_fd_key(&key) else {
                continue;
            };
            if parsed.pid == pid && parsed.generation == generation {
                self.file_bulk_read_fast_fd_stats
                    .delete(&key)
                    .map_err(|error| {
                        LoaderError::new("sweep_file_bulk_read_fast_fds", error.to_string())
                    })?;
            }
        }
        Ok(())
    }

    pub fn max_tracked_processes(&self) -> u32 {
        self.tracked_traces.max_entries()
    }

    pub fn event_poll_fd(&self) -> Result<RawFd, LoaderError> {
        let fd = self.event_buffer.epoll_fd();
        if fd < 0 {
            return Err(LoaderError::new(
                "event_poll_fd",
                format!("event buffer returned invalid epoll fd {fd}"),
            ));
        }
        Ok(fd)
    }

    pub fn lookup_pending_tls_payload_op(
        &self,
        tid: u32,
    ) -> Result<Option<PendingTlsPayloadOp>, LoaderError> {
        tls::lookup_pending_payload_op(
            &self.pending_tls_payload_ops_by_namespace,
            &self.pending_tls_payload_ops,
            tid,
        )
    }

    pub fn tls_payload_diagnostics(&self) -> Result<TlsPayloadDiagnostics, LoaderError> {
        tls::read_tls_payload_diagnostics(&self.payload_tls_diagnostics)
    }

    pub fn attach_go_tls_executable(&mut self, binary_path: &Path) -> Result<bool, LoaderError> {
        let outcome = tls::attach_go_tls_programs(&mut self._object, binary_path)?;
        let GoTlsAttachOutcome::Attached(links) = outcome else {
            return Ok(false);
        };
        for (link, program_name) in links {
            self._links.push(link);
            self.attached_programs.push(program_name);
        }
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EventTransportDiagnostics {
    reserve_fail: u64,
    output_fail: u64,
    output_fail_bytes: u64,
}

fn read_event_transport_diagnostics(
    map: &MapHandle,
) -> Result<EventTransportDiagnostics, LoaderError> {
    Ok(EventTransportDiagnostics {
        reserve_fail: read_event_transport_counter(map, 0)?,
        output_fail: read_event_transport_counter(map, 1)?,
        output_fail_bytes: read_event_transport_counter(map, 2)?,
    })
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

fn configure_collector_pid_namespace(pid_namespace: &MapHandle) -> Result<(), LoaderError> {
    let namespace = read_collector_pid_namespace()?;
    write_pid_namespace_map(pid_namespace, namespace, "collector_pid_namespace")
}

fn read_collector_pid_namespace() -> Result<PidNamespace, LoaderError> {
    let pid = std::process::id();
    read_pid_namespace_for_pid(pid)
}

fn read_pid_namespace_for_pid(pid: u32) -> Result<PidNamespace, LoaderError> {
    let path = format!("/proc/{pid}/ns/pid");
    let metadata = std::fs::metadata(&path)
        .map_err(|error| LoaderError::new("collector_pid_namespace", error.to_string()))?;
    Ok(PidNamespace {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

fn write_pid_namespace_map(
    pid_namespace: &MapHandle,
    namespace: PidNamespace,
    stage: &'static str,
) -> Result<(), LoaderError> {
    let key = ACTIVE_PID_NAMESPACE_KEY.to_ne_bytes();
    let mut value = [0_u8; PID_NAMESPACE_VALUE_SIZE];
    value[0..PID_NAMESPACE_FIELD_SIZE].copy_from_slice(&namespace.dev.to_ne_bytes());
    value[PID_NAMESPACE_FIELD_SIZE..PID_NAMESPACE_VALUE_SIZE]
        .copy_from_slice(&namespace.ino.to_ne_bytes());
    pid_namespace
        .update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new(stage, error.to_string()))
}

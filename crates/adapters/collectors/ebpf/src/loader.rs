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
use std::ffi::OsStr;
use std::os::fd::RawFd;
use std::path::Path;
use std::rc::Rc;

use config_core::daemon::{EbpfCollectorConfig, PayloadConfig};
use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder, RingBuffer};
use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessSuppressedFd};

pub use attach_plan::AttachPlan;
use attach_plan::{configure_program_autoload, effective_config_for_attach_plan};
use object::{build_ring_buffer, map_handle, resize_map, ring_buffer_max_bytes};
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
pub struct PidNamespace {
    pub dev: u64,
    pub ino: u64,
}

pub struct EbpfProgramLoader {
    config: EbpfCollectorConfig,
    payload: PayloadConfig,
}

pub struct EbpfRuntime {
    _object: Object,
    _links: Vec<Link>,
    attached_programs: Vec<String>,
    attached_capabilities: BTreeSet<Capability>,
    pid_namespace: MapHandle,
    tracked_traces: MapHandle,
    process_generations: MapHandle,
    suppressed_fds: MapHandle,
    suppressed_fd_index: MapHandle,
    suppressed_fd_index_slots_per_process: u32,
    pending_tls_payload_ops: MapHandle,
    pending_tls_payload_ops_by_namespace: MapHandle,
    payload_tls_diagnostics: MapHandle,
    events: Rc<RefCell<Vec<Vec<u8>>>>,
    ring_buffer: RingBuffer<'static>,
    last_raw_sample_count: usize,
}

impl EbpfProgramLoader {
    pub fn new(config: EbpfCollectorConfig, payload: PayloadConfig) -> Self {
        Self { config, payload }
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
        resize_map(
            &mut open_object,
            "events",
            ring_buffer_max_bytes(&self.config, &effective_payload),
        )?;
        configure_program_autoload(&mut open_object, attach_plan)?;

        let object = open_object
            .load()
            .map_err(|error| LoaderError::new("load_object", error.to_string()))?;
        EbpfRuntime::from_object(object, &self.config, &effective_payload, attach_plan)
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
        let suppressed_fds = map_handle(&object, "suppressed_fds", "suppressed_fds")?;
        let suppressed_fd_index =
            map_handle(&object, "suppressed_fd_index", "suppressed_fd_index")?;
        let pid_namespace = map_handle(&object, "pid_namespace", "pid_namespace_map")?;
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
        let events_map = map_handle(&object, "events", "ring_buffer")?;

        let events = Rc::new(RefCell::new(Vec::new()));
        let ring_buffer = build_ring_buffer(&events_map, Rc::clone(&events))?;
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
            pid_namespace,
            tracked_traces,
            process_generations,
            suppressed_fds,
            suppressed_fd_index,
            suppressed_fd_index_slots_per_process: config.suppressed_fd_index_slots_per_process,
            pending_tls_payload_ops,
            pending_tls_payload_ops_by_namespace,
            payload_tls_diagnostics,
            events,
            ring_buffer,
            last_raw_sample_count: 0,
        })
    }

    pub fn poll_events(&mut self) -> Result<Vec<KernelEvent>, LoaderError> {
        self.ring_buffer
            .consume()
            .map_err(|error| LoaderError::new("consume_ring_buffer", error.to_string()))?;
        let raw_events = std::mem::take(&mut *self.events.borrow_mut());
        self.last_raw_sample_count = raw_events.len();
        raw_events
            .into_iter()
            .map(|raw| decode_kernel_event(&raw))
            .collect()
    }

    pub fn track_pid(
        &self,
        identity: &ProcessIdentity,
        trace_id: TraceId,
    ) -> Result<(), LoaderError> {
        let pid = identity.pid;
        let key = pid.to_ne_bytes();
        let value = trace_id.get().to_ne_bytes();
        self.tracked_traces
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid", error.to_string()))?;
        self.process_generations
            .update(&key, &identity.generation.to_ne_bytes(), MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid_generation", error.to_string()))
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

    pub fn set_pid_namespace(&self, namespace: PidNamespace) -> Result<(), LoaderError> {
        let key = ACTIVE_PID_NAMESPACE_KEY.to_ne_bytes();
        let mut value = [0_u8; PID_NAMESPACE_VALUE_SIZE];
        value[0..PID_NAMESPACE_FIELD_SIZE].copy_from_slice(&namespace.dev.to_ne_bytes());
        value[PID_NAMESPACE_FIELD_SIZE..PID_NAMESPACE_VALUE_SIZE]
            .copy_from_slice(&namespace.ino.to_ne_bytes());
        self.pid_namespace
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("set_pid_namespace", error.to_string()))
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

    pub fn max_tracked_processes(&self) -> u32 {
        self.tracked_traces.max_entries()
    }

    pub fn event_poll_fd(&self) -> Result<RawFd, LoaderError> {
        let fd = self.ring_buffer.epoll_fd();
        if fd < 0 {
            return Err(LoaderError::new(
                "event_poll_fd",
                format!("ring buffer returned invalid epoll fd {fd}"),
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

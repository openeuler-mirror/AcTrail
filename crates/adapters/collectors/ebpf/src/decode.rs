//! Decoding of kernel eBPF observations into collector contracts.

#[path = "decode/fd_io.rs"]
mod fd_io;
#[path = "decode/file_path/mod.rs"]
mod file_path;
#[path = "decode/payload.rs"]
mod payload;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{
    HostProcessCoordinates, NamespaceProcessCoordinates, ProcessObservation,
};

use crate::loader::{KernelEndpoint, KernelObservationEvent};
use crate::maps::BindingStateMap;

pub const PROC_EVENT_FORK: u32 = 1;
pub const PROC_EVENT_EXEC: u32 = 2;
pub const PROC_EVENT_EXIT: u32 = 3;
pub const PROC_EVENT_SIGNAL: u32 = 4;
pub const NET_EVENT_CONNECT: u32 = 100;
pub const NET_EVENT_ACCEPT: u32 = 101;
pub const NET_EVENT_SEND: u32 = 102;
pub const NET_EVENT_RECV: u32 = 103;
pub const NET_EVENT_BIND: u32 = 104;
pub const NET_EVENT_LISTEN: u32 = 105;
pub const FILE_EVENT_OPEN: u32 = 300;
pub const FILE_EVENT_UNLINK: u32 = 301;
pub const FILE_EVENT_RENAME: u32 = 302;
pub const FILE_EVENT_MKDIR: u32 = 303;
pub const FILE_EVENT_RMDIR: u32 = 304;
pub const FILE_EVENT_TRUNCATE: u32 = 305;
pub const FILE_EVENT_MMAP: u32 = 306;
pub const FILE_EVENT_CONTEXT: u32 = 307;
pub const FILE_EVENT_READ_SUMMARY: u32 = 308;
const NET_SYSCALL_SOCKET: u32 = 1;
const NET_SYSCALL_FD_IO: u32 = 2;
const NET_SYSCALL_FD_IO_WRITEV: u32 = 3;
const PROC_COORD_TRACEPOINT_SIGNAL_GENERATE: u32 = 1;

pub(crate) use file_path::{FdIpcKind, FileTracker};
pub use payload::{
    SOCKET_PAYLOAD_DIRECTION_INBOUND, SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
    SOCKET_PAYLOAD_SYSCALL_READ, SOCKET_PAYLOAD_SYSCALL_RECVFROM, SOCKET_PAYLOAD_SYSCALL_SENDMSG,
    SOCKET_PAYLOAD_SYSCALL_SENDTO, SOCKET_PAYLOAD_SYSCALL_WRITE, SOCKET_PAYLOAD_SYSCALL_WRITEV,
    SocketPayloadCompletion, TlsDiagnosticEvent, TlsPayloadCaptureRequest, TlsPayloadCompletion,
    TlsPayloadDirectCapture, decode_socket_payload, decode_socket_payload_completion,
    decode_stdio_payload, decode_tls_capture_request, decode_tls_completion, decode_tls_diagnostic,
    decode_tls_direct_capture,
};

pub(crate) fn decode_file_path(
    event: crate::loader::KernelFilePathEvent,
    bindings: &BindingStateMap,
    tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    file_path::decode(event, bindings, tracker)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeError {
    pub stage: String,
    pub message: String,
}

impl DecodeError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

pub(crate) fn decode_observation(
    event: KernelObservationEvent,
    bindings: &mut BindingStateMap,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let lifecycle_requested =
        bindings.trace_has_capability(event.trace_id, &Capability::ProcLifecycle);
    match event.kind {
        PROC_EVENT_FORK => {
            maybe_lifecycle_event(lifecycle_requested, decode_fork(event, bindings)?)
        }
        PROC_EVENT_EXEC => {
            maybe_lifecycle_event(lifecycle_requested, decode_exec(event, bindings)?)
        }
        PROC_EVENT_EXIT => {
            maybe_lifecycle_event(lifecycle_requested, decode_exit(event, bindings)?)
        }
        PROC_EVENT_SIGNAL => {
            maybe_lifecycle_event(lifecycle_requested, decode_signal(event, bindings)?)
        }
        NET_EVENT_CONNECT | NET_EVENT_ACCEPT | NET_EVENT_SEND | NET_EVENT_RECV | NET_EVENT_BIND
        | NET_EVENT_LISTEN => decode_net(event, bindings, file_tracker),
        other => Err(DecodeError::new(
            "decode_observation",
            format!("unknown kernel event kind {other}"),
        )),
    }
}

fn maybe_lifecycle_event(
    enabled: bool,
    event: Option<RawCollectorEvent>,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if enabled { Ok(event) } else { Ok(None) }
}

fn decode_fork(
    event: KernelObservationEvent,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let parent = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("parent_identity", error))?;
    let child = resolve_event_observation(
        event.trace_id,
        event.aux,
        event.aux_host_pid,
        event.aux_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("fork_identity", error))?;
    bindings.track_with_map_pid(
        event.trace_id,
        child.clone(),
        event.aux,
        event.aux_generation,
    );

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process: child,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            operation: "fork".to_string(),
            parent: Some(parent),
            metadata: BTreeMap::new(),
        },
    }))
}

fn decode_exec(
    event: KernelObservationEvent,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("exec_identity", error))?;
    bindings.track_with_map_pid(
        event.trace_id,
        observation.clone(),
        event.pid,
        event.pid_generation,
    );
    let mut metadata = BTreeMap::new();
    if let Some(exec_filename) = event.exec_filename {
        metadata.insert("executable".to_string(), exec_filename.path.clone());
        metadata.insert("exec_filename".to_string(), exec_filename.path);
        metadata.insert(
            "exec_filename_source".to_string(),
            "sched_process_exec".to_string(),
        );
        if exec_filename.truncated {
            metadata.insert("exec_filename_truncated".to_string(), "true".to_string());
        }
    }

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            operation: "exec".to_string(),
            parent: None,
            metadata,
        },
    }))
}

fn decode_exit(
    event: KernelObservationEvent,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("exit_identity", error))?;

    let mut metadata = BTreeMap::new();
    if event.result != 0 {
        metadata.insert("exit_code".to_string(), event.aux.to_string());
    }

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            operation: "exit".to_string(),
            parent: None,
            metadata,
        },
    }))
}

fn decode_signal(
    event: KernelObservationEvent,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("process_coordination_identity", error))?;
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), "signal".to_string()),
        ("result".to_string(), event.result.to_string()),
        (
            "syscall".to_string(),
            process_coordination_syscall(event.aux).to_string(),
        ),
    ]);
    metadata.insert("target_pid".to_string(), event.requested_size.to_string());
    metadata.insert("signal".to_string(), event.fd.to_string());
    if event.reserved != 0 {
        metadata.insert("target_group".to_string(), event.reserved.to_string());
    }
    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            operation: "signal".to_string(),
            parent: None,
            metadata,
        },
    }))
}

fn decode_net(
    event: KernelObservationEvent,
    bindings: &mut BindingStateMap,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("net_identity", error))?;

    let local = format_endpoint(&event.local);
    let remote = format_endpoint(&event.remote);

    let endpoint_source = if local.is_some() || remote.is_some() {
        "syscall_sockaddr"
    } else if event.aux == NET_SYSCALL_SOCKET {
        "unresolved_socket_syscall"
    } else {
        "unresolved_fd_io"
    };

    let (operation, direction) = net_operation(event.kind);
    if matches!(
        endpoint_source,
        "unresolved_fd_io" | "unresolved_socket_syscall"
    ) {
        let (operation, direction) = fd_io::operation(event.kind, event.aux);
        if let Some(event) = fd_io::decode(
            event.clone(),
            bindings,
            observation.clone(),
            operation,
            direction,
            file_tracker,
        )? {
            return Ok(Some(event));
        }
        if endpoint_source == "unresolved_fd_io" {
            return Ok(None);
        }
    }
    if !bindings.trace_has_capability(event.trace_id, &Capability::NetTransport) {
        return Ok(None);
    }

    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("direction".to_string(), direction.to_string()),
        ("fd".to_string(), event.fd.to_string()),
        ("result".to_string(), event.result.to_string()),
        (
            "syscall_family".to_string(),
            net_syscall_family(event.aux).to_string(),
        ),
        ("endpoint_source".to_string(), endpoint_source.to_string()),
    ]);
    if endpoint_source == "unresolved_socket_syscall" {
        metadata.insert("endpoint_unresolved".to_string(), "true".to_string());
    }
    if event.requested_size > 0 {
        metadata.insert(
            "requested_size".to_string(),
            event.requested_size.to_string(),
        );
    }

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Net {
            transport: net_transport(event.kind).to_string(),
            local,
            remote,
            size: net_size(event.kind, event.result),
            result: Some(event.result),
            metadata,
        },
    }))
}

fn net_operation(kind: u32) -> (&'static str, &'static str) {
    match kind {
        NET_EVENT_CONNECT => ("connect", "outbound"),
        NET_EVENT_ACCEPT => ("accept", "inbound"),
        NET_EVENT_SEND => ("send", "outbound"),
        NET_EVENT_RECV => ("recv", "inbound"),
        NET_EVENT_BIND => ("bind", "local"),
        NET_EVENT_LISTEN => ("listen", "local"),
        _ => ("unknown", "unknown"),
    }
}

fn process_coordination_syscall(raw: u32) -> &'static str {
    match raw {
        PROC_COORD_TRACEPOINT_SIGNAL_GENERATE => "signal_generate",
        _ => "unknown",
    }
}

fn net_size(kind: u32, result: i32) -> Option<u64> {
    if !matches!(kind, NET_EVENT_SEND | NET_EVENT_RECV) || result < 0 {
        return None;
    }
    Some(result as u64)
}

fn net_transport(kind: u32) -> &'static str {
    match kind {
        NET_EVENT_BIND | NET_EVENT_LISTEN | NET_EVENT_CONNECT | NET_EVENT_ACCEPT => "tcp",
        _ => "unknown",
    }
}

fn net_syscall_family(raw: u32) -> &'static str {
    match raw {
        NET_SYSCALL_SOCKET => "socket",
        NET_SYSCALL_FD_IO => "fd_io",
        NET_SYSCALL_FD_IO_WRITEV => "fd_io_writev",
        _ => "unknown",
    }
}

fn format_endpoint(endpoint: &KernelEndpoint) -> Option<String> {
    let port = u16::from_be(endpoint.port_be);
    match endpoint.family as i32 {
        libc::AF_INET => {
            let ip = Ipv4Addr::from(endpoint.addr4_be.to_ne_bytes());
            if ip.is_unspecified() && port == 0 {
                return None;
            }
            Some(format!("{ip}:{port}"))
        }
        libc::AF_INET6 => {
            let ip = Ipv6Addr::from(endpoint.addr6);
            if ip.is_unspecified() && port == 0 {
                return None;
            }
            Some(format!("[{ip}]:{port}"))
        }
        _ => None,
    }
}

pub(crate) fn resolve_bound_event_observation(
    trace_id: TraceId,
    map_pid: u32,
    generation: u64,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    resolve_event_observation(trace_id, map_pid, 0, generation, bindings)
}

pub(crate) fn resolve_event_observation(
    trace_id: TraceId,
    namespace_pid: u32,
    host_pid: u32,
    kernel_start_time: u64,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    if let Some(observation) = bindings
        .tracked_event_observation(trace_id, namespace_pid, kernel_start_time)
        .cloned()
    {
        return Ok(observation);
    }
    let namespace = bindings
        .trace_pid_namespace(trace_id)
        .ok_or_else(|| format!("trace {} has no PID namespace binding", trace_id.get()))?;
    let namespace = NamespaceProcessCoordinates::new(namespace.clone(), namespace_pid, 0);
    let mut observation = ProcessObservation::namespace(namespace);
    if host_pid != 0 {
        let mut host = HostProcessCoordinates::new(host_pid, 0);
        if kernel_start_time != 0 {
            host = host.with_start_boottime_ns(kernel_start_time);
        }
        observation.host = Some(host);
    }
    Ok(observation)
}

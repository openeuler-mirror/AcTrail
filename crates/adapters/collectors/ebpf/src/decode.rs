//! Decoding of kernel eBPF observations into collector contracts.

#[path = "decode/clock.rs"]
mod clock;
#[path = "decode/fd_io.rs"]
mod fd_io;
#[path = "decode/file_path/mod.rs"]
mod file_path;
#[path = "decode/payload.rs"]
mod payload;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{
    HostProcessCoordinates, NamespaceProcessCoordinates, ProcessObservation,
};

use crate::loader::{
    KernelEndpoint, KernelEndpointRole, KernelEndpointWithRole, KernelEventIdentity,
    KernelExecPayload, KernelExitPayload, KernelFdIoPayload, KernelForkPayload,
    KernelNetworkOperation, KernelNetworkPayload, KernelObservationCommon, KernelObservationEvent,
    KernelObservationPayload, KernelSignalPayload,
};
use crate::maps::BindingStateMap;

pub const PROC_EVENT_FORK: u32 = 1;
pub const PROC_EVENT_EXEC: u32 = 2;
pub const PROC_EVENT_EXIT: u32 = 3;
pub const PROC_EVENT_SIGNAL: u32 = 4;
pub const NET_EVENT_CONNECT: u32 = 100;
pub const NET_EVENT_ACCEPT: u32 = 101;
pub const FD_IO_EVENT_SEND: u32 = 102;
pub const FD_IO_EVENT_RECV: u32 = 103;
pub const NET_EVENT_BIND: u32 = 104;
pub const NET_EVENT_LISTEN: u32 = 105;
pub const NET_EVENT_CLOSE: u32 = 106;
pub const NET_EVENT_SHUTDOWN: u32 = 107;
pub const SOCKET_FD_RELEASE_EVENT: u32 = 108;
pub const FILE_EVENT_OPEN: u32 = 300;
pub const FILE_EVENT_UNLINK: u32 = 301;
pub const FILE_EVENT_RENAME: u32 = 302;
pub const FILE_EVENT_MKDIR: u32 = 303;
pub const FILE_EVENT_RMDIR: u32 = 304;
pub const FILE_EVENT_TRUNCATE: u32 = 305;
pub const FILE_EVENT_MMAP: u32 = 306;
pub const FILE_EVENT_CONTEXT: u32 = 307;
pub const FILE_EVENT_READ_SUMMARY: u32 = 308;
const SYSCALL_FAMILY_SOCKET: u32 = 1;
const SYSCALL_FAMILY_FD_IO: u32 = 2;
const SYSCALL_FAMILY_FD_IO_WRITEV: u32 = 3;
// Should be same with enum actrail_fd_category in bpf/fd/types.h.
const FD_CATEGORY_NET: u32 = 1;
const FD_CATEGORY_IPC_UNIX_SOCKET: u32 = 2;
const FD_CATEGORY_IPC_PIPE: u32 = 3;
const FD_CATEGORY_FILE: u32 = 4;

use file_path::FdIpcKind;
pub(crate) use file_path::FileTracker;
pub use payload::{
    SOCKET_PAYLOAD_DIRECTION_INBOUND, SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
    SOCKET_PAYLOAD_SYSCALL_READ, SOCKET_PAYLOAD_SYSCALL_RECVFROM, SOCKET_PAYLOAD_SYSCALL_SENDMSG,
    SOCKET_PAYLOAD_SYSCALL_SENDTO, SOCKET_PAYLOAD_SYSCALL_WRITE, SOCKET_PAYLOAD_SYSCALL_WRITEV,
    SocketPayloadCompletion, TlsDiagnosticEvent, TlsPayloadCaptureRequest, TlsPayloadCompletion,
    TlsPayloadDirectCapture, decode_socket_fd_release, decode_socket_payload,
    decode_socket_payload_completion, decode_stdio_payload, decode_tls_capture_request,
    decode_tls_completion, decode_tls_diagnostic, decode_tls_direct_capture,
};
pub(crate) use payload::{
    STDIO_PAYLOAD_DIRECTION_OUTBOUND, STDIO_PAYLOAD_FLAG_STAGED, STDIO_PAYLOAD_SYSCALL_WRITE,
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
    event: &KernelObservationEvent,
    bindings: &mut BindingStateMap,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let lifecycle_requested =
        bindings.trace_has_capability(event.common.trace_id, &Capability::ProcLifecycle);
    match &event.payload {
        KernelObservationPayload::Fork(payload) => maybe_lifecycle_event(
            lifecycle_requested,
            decode_fork(&event.common, payload, bindings)?,
        ),
        KernelObservationPayload::Exec(payload) => maybe_lifecycle_event(
            lifecycle_requested,
            decode_exec(&event.common, payload, bindings)?,
        ),
        KernelObservationPayload::Exit(payload) => maybe_lifecycle_event(
            lifecycle_requested,
            decode_exit(&event.common, payload, bindings)?,
        ),
        KernelObservationPayload::SignalGenerate(payload) => maybe_lifecycle_event(
            lifecycle_requested,
            decode_signal(&event.common, payload, bindings)?,
        ),
        KernelObservationPayload::SocketFdRelease(_) => Ok(None),
        KernelObservationPayload::Network(payload) => {
            decode_connection_event(&event.common, payload, bindings)
        }
        KernelObservationPayload::FdIo(payload) => {
            decode_fd_io_event(&event.common, payload, bindings, file_tracker)
        }
    }
}

fn maybe_lifecycle_event(
    enabled: bool,
    event: Option<RawCollectorEvent>,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if enabled { Ok(event) } else { Ok(None) }
}

fn decode_fork(
    common: &KernelObservationCommon,
    payload: &KernelForkPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let parent = resolve_fork_observation(common.trace_id, &payload.parent, bindings)
        .map_err(|error| DecodeError::new("parent_identity", error))?;
    let child = resolve_fork_observation(common.trace_id, &common.subject, bindings)
        .map_err(|error| DecodeError::new("fork_identity", error))?;
    bindings.track_with_kernel_tgid(
        common.trace_id,
        child.clone(),
        common.subject.binding_tgid(),
        common.subject.start_boottime_ns,
    );

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
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

pub(crate) fn fork_parent_observation(
    event: &KernelObservationEvent,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, DecodeError> {
    let KernelObservationPayload::Fork(payload) = &event.payload else {
        return Err(DecodeError::new("parent_identity", "event is not fork"));
    };
    resolve_fork_observation(event.common.trace_id, &payload.parent, bindings)
        .map_err(|error| DecodeError::new("parent_identity", error))
}

pub(crate) fn fork_child_observation(
    event: &KernelObservationEvent,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, DecodeError> {
    let KernelObservationPayload::Fork(_) = &event.payload else {
        return Err(DecodeError::new("fork_identity", "event is not fork"));
    };
    resolve_fork_observation(event.common.trace_id, &event.common.subject, bindings)
        .map_err(|error| DecodeError::new("fork_identity", error))
}

fn resolve_fork_observation(
    trace_id: TraceId,
    identity: &KernelEventIdentity,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    if identity.observer_namespace_tgid == 0
        || identity.kernel_tgid == 0
        || identity.start_boottime_ns == 0
    {
        return Err(
            "fork event requires observer-namespace TGID, kernel TGID, and start boottime"
                .to_string(),
        );
    }
    if let Some(observation) = bindings
        .tracked_event_observation(
            trace_id,
            identity.binding_tgid(),
            identity.start_boottime_ns,
        )
        .cloned()
    {
        return Ok(observation);
    }
    Ok(ProcessObservation::host(
        HostProcessCoordinates::new(identity.observer_namespace_tgid, 0)
            .with_start_boottime_ns(identity.start_boottime_ns),
    ))
}

fn decode_exec(
    common: &KernelObservationCommon,
    payload: &KernelExecPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("exec_identity", error))?;
    bindings.track_with_kernel_tgid(
        common.trace_id,
        observation.clone(),
        common.subject.binding_tgid(),
        common.subject.start_boottime_ns,
    );
    let mut metadata = BTreeMap::new();
    if let Some(exec_filename) = &payload.filename {
        metadata.insert("executable".to_string(), exec_filename.path.clone());
        metadata.insert("exec_filename".to_string(), exec_filename.path.clone());
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
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
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
    common: &KernelObservationCommon,
    payload: &KernelExitPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("exit_identity", error))?;

    let mut metadata = BTreeMap::new();
    if let Some(exit_code) = payload.exit_code {
        metadata.insert("exit_code".to_string(), exit_code.to_string());
    }

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
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
    common: &KernelObservationCommon,
    payload: &KernelSignalPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("process_coordination_identity", error))?;
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), "signal".to_string()),
        ("result".to_string(), payload.signal_result.to_string()),
        ("syscall".to_string(), "signal_generate".to_string()),
    ]);
    metadata.insert(
        "target_kernel_tid".to_string(),
        payload.target_kernel_tid.to_string(),
    );
    metadata.insert("signal".to_string(), payload.signal.to_string());
    if payload.target_group != 0 {
        metadata.insert("target_group".to_string(), payload.target_group.to_string());
    }
    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
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

fn decode_connection_event(
    common: &KernelObservationCommon,
    event: &KernelNetworkPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("net_identity", error))?;

    // 连接生命周期事件（connect/accept/bind/listen/close/shutdown）只可能来自
    // socket，不可能是文件或管道 I/O，无需 fd_io 分类。
    if !bindings.trace_has_capability(common.trace_id, &Capability::NetTransport) {
        return Ok(None);
    }
    Ok(Some(
        NetProjection::network(event).build(common, observation),
    ))
}

fn decode_fd_io_event(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    bindings: &mut BindingStateMap,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("net_identity", error))?;

    let projection = NetProjection::fd_io(event);
    let endpoint_source = projection.endpoint_source();
    let (operation, direction) = fd_io::operation(event.operation, event.syscall_family);
    // 内核 fd_table 的类别码是权威分类；即使 sendto/recvfrom 携带一个可解析的
    // sockaddr，也不能把 FILE 或 IPC descriptor 伪装成网络事件。
    match fd_io_category(event.fd_category) {
        Some(FD_CATEGORY_FILE) => {
            return fd_io::decode_file(
                common,
                event,
                bindings,
                observation,
                operation,
                direction,
                file_tracker,
            );
        }
        Some(category @ (FD_CATEGORY_IPC_PIPE | FD_CATEGORY_IPC_UNIX_SOCKET)) => {
            let classified_kind = if category == FD_CATEGORY_IPC_PIPE {
                FdIpcKind::Pipe
            } else {
                FdIpcKind::UnixSocket
            };
            return fd_io::decode_ipc(
                common,
                event,
                bindings,
                observation,
                operation,
                direction,
                file_tracker,
                Some(classified_kind),
            );
        }
        Some(FD_CATEGORY_NET) => {
            // 真 socket：直接走 Net 观测（无端点也带 endpoint_unresolved 保留）。
        }
        Some(_) => return Ok(None),
        None => {
            if matches!(
                endpoint_source,
                "unresolved_fd_io" | "unresolved_socket_syscall"
            ) {
                if let Some(event) = fd_io::decode(
                    common,
                    event,
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
        }
    }
    if !bindings.trace_has_capability(common.trace_id, &Capability::NetTransport) {
        return Ok(None);
    }
    Ok(Some(projection.build(common, observation)))
}

struct NetProjection<'a> {
    operation: &'static str,
    direction: &'static str,
    transport: &'static str,
    syscall_result: i32,
    fd: u32,
    syscall_family: u32,
    operation_flags: u32,
    requested_size: u64,
    endpoint: &'a Option<KernelEndpointWithRole>,
    has_io_size: bool,
}

impl<'a> NetProjection<'a> {
    fn network(event: &'a KernelNetworkPayload) -> Self {
        let (operation, direction, transport) = match event.operation {
            KernelNetworkOperation::Connect => ("connect", "outbound", "tcp"),
            KernelNetworkOperation::Accept => ("accept", "inbound", "tcp"),
            KernelNetworkOperation::Bind => ("bind", "local", "tcp"),
            KernelNetworkOperation::Listen => ("listen", "local", "tcp"),
            KernelNetworkOperation::Close => ("close", "unknown", "unknown"),
            KernelNetworkOperation::Shutdown => ("shutdown", "unknown", "unknown"),
        };
        Self {
            operation,
            direction,
            transport,
            syscall_result: event.syscall_result,
            fd: event.fd,
            syscall_family: event.syscall_family,
            operation_flags: event.operation_flags,
            requested_size: 0,
            endpoint: &event.endpoint,
            has_io_size: false,
        }
    }

    fn fd_io(event: &'a KernelFdIoPayload) -> Self {
        let (operation, direction) = match event.operation {
            crate::loader::KernelFdIoOperation::Send => ("send", "outbound"),
            crate::loader::KernelFdIoOperation::Recv => ("recv", "inbound"),
        };
        Self {
            operation,
            direction,
            transport: "unknown",
            syscall_result: event.syscall_result,
            fd: event.fd,
            syscall_family: event.syscall_family,
            operation_flags: 0,
            requested_size: event.requested_size,
            endpoint: &event.endpoint,
            has_io_size: true,
        }
    }

    fn endpoint_source(&self) -> &'static str {
        if self.endpoint.is_some() {
            "syscall_sockaddr"
        } else if self.syscall_family == SYSCALL_FAMILY_SOCKET {
            "unresolved_socket_syscall"
        } else {
            "unresolved_fd_io"
        }
    }

    fn build(
        &self,
        common: &KernelObservationCommon,
        observation: ProcessObservation,
    ) -> RawCollectorEvent {
        let (local, remote) = format_typed_endpoint(self.endpoint);
        let endpoint_source = self.endpoint_source();
        let mut metadata = BTreeMap::from([
            ("operation".to_string(), self.operation.to_string()),
            ("direction".to_string(), self.direction.to_string()),
            ("fd".to_string(), self.fd.to_string()),
            ("result".to_string(), self.syscall_result.to_string()),
            (
                "syscall_family".to_string(),
                syscall_family(self.syscall_family).to_string(),
            ),
            ("endpoint_source".to_string(), endpoint_source.to_string()),
        ]);
        if endpoint_source == "unresolved_socket_syscall" {
            metadata.insert("endpoint_unresolved".to_string(), "true".to_string());
        }
        if self.operation == "shutdown" {
            metadata.insert("shutdown_how".to_string(), self.operation_flags.to_string());
        }
        if self.requested_size > 0 {
            metadata.insert(
                "requested_size".to_string(),
                self.requested_size.to_string(),
            );
        }
        RawCollectorEvent {
            envelope: RawEventEnvelope {
                trace_id: Some(common.trace_id),
                observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
                process: observation,
                collector: CollectorName::new("ebpf"),
            },
            payload: RawObservationPayload::Net {
                transport: self.transport.to_string(),
                local,
                remote,
                size: (self.has_io_size && self.syscall_result >= 0)
                    .then_some(self.syscall_result as u64),
                result: Some(self.syscall_result),
                metadata,
            },
        }
    }
}

/// 从 SEND/RECV 事件里读出内核 fd_table 的类别码；0 表示 fd 不在表内（无类别）。
fn fd_io_category(category: u32) -> Option<u32> {
    if category != 0 { Some(category) } else { None }
}

fn syscall_family(raw: u32) -> &'static str {
    match raw {
        SYSCALL_FAMILY_SOCKET => "socket",
        SYSCALL_FAMILY_FD_IO => "fd_io",
        SYSCALL_FAMILY_FD_IO_WRITEV => "fd_io_writev",
        _ => "unknown",
    }
}

fn format_typed_endpoint(
    endpoint: &Option<KernelEndpointWithRole>,
) -> (Option<String>, Option<String>) {
    match endpoint {
        Some(endpoint) if endpoint.role == KernelEndpointRole::Local => {
            (format_endpoint(&endpoint.endpoint), None)
        }
        Some(endpoint) => (None, format_endpoint(&endpoint.endpoint)),
        None => (None, None),
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
    kernel_tgid: u32,
    generation: u64,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    resolve_event_observation(trace_id, kernel_tgid, 0, generation, bindings)
}

pub(crate) fn resolve_typed_event_observation(
    common: &KernelObservationCommon,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    let identity = common.subject;
    if identity.kernel_tgid == 0 || identity.start_boottime_ns == 0 {
        return Err("typed event requires kernel TGID and start boottime".to_string());
    }
    if let Some(observation) = bindings
        .tracked_event_observation(
            common.trace_id,
            identity.binding_tgid(),
            identity.start_boottime_ns,
        )
        .cloned()
    {
        return Ok(observation);
    }
    if identity.observer_namespace_tgid == 0 {
        return Err("unbound typed event requires observer-namespace TGID".to_string());
    }
    Ok(ProcessObservation::host(
        HostProcessCoordinates::new(identity.observer_namespace_tgid, 0)
            .with_start_boottime_ns(identity.start_boottime_ns),
    ))
}

pub(crate) fn event_kernel_tgid(namespace_pid: u32, host_pid: u32) -> u32 {
    if host_pid != 0 {
        host_pid
    } else {
        namespace_pid
    }
}

pub(crate) fn resolve_event_observation(
    trace_id: TraceId,
    namespace_pid: u32,
    host_pid: u32,
    kernel_start_time: u64,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    let kernel_tgid = event_kernel_tgid(namespace_pid, host_pid);
    if let Some(observation) = bindings
        .tracked_event_observation(trace_id, kernel_tgid, kernel_start_time)
        .cloned()
    {
        return Ok(observation);
    }
    let namespace = bindings
        .trace_pid_namespace(trace_id)
        .ok_or_else(|| format!("trace {} has no PID namespace binding", trace_id.get()))?;
    let namespace = NamespaceProcessCoordinates::new(namespace.clone(), namespace_pid, 0);
    Ok(ProcessObservation::namespace(namespace))
}

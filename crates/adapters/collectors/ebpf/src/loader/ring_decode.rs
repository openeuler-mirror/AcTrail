//! Ring-buffer record decoding.

#[path = "ring_decode/file_path.rs"]
mod file_path;
#[path = "ring_decode/payload.rs"]
mod payload;

use model_core::ids::TraceId;

use crate::loader::LoaderError;

use file_path::decode_file_path_event;
pub use payload::{
    KernelSocketPayloadCompletionEvent, KernelSocketPayloadEvent,
    KernelStdioPayloadCompletionEvent, KernelStdioPayloadEvent, KernelTlsCaptureRequestEvent,
    KernelTlsCompletionEvent, KernelTlsDiagnosticEvent, KernelTlsDirectCaptureEvent,
};
use payload::{
    decode_socket_payload_completion_event, decode_socket_payload_event,
    decode_stdio_payload_completion_event, decode_stdio_payload_event,
    decode_tls_capture_request_event, decode_tls_completion_event, decode_tls_diagnostic_event,
    decode_tls_direct_capture_event,
};

use super::abi::{
    EVENT_ABI_REVISION, EVENT_HEADER_SIZE, EXEC_EVENT_FILENAME_FLAGS_OFFSET,
    EXEC_EVENT_FILENAME_OFFSET, EXEC_EVENT_FILENAME_SIZE_OFFSET, EXEC_FILENAME_ABI_MAX_BYTES,
    EXEC_FILENAME_FLAG_TRUNCATED, FD_IO_EVENT_SIZE, KERNEL_ENDPOINT_SIZE,
    LAUNCH_BINDING_FAILURE_EVENT_SIZE, NETWORK_EVENT_SIZE, PROC_EXEC_EVENT_KIND,
    PROC_EXIT_EVENT_KIND, PROC_FORK_EVENT_KIND, PROC_SIGNAL_EVENT_KIND, PROCESS_EXEC_EVENT_SIZE,
    PROCESS_EXIT_EVENT_SIZE, PROCESS_FORK_EVENT_SIZE, PROCESS_SIGNAL_EVENT_SIZE,
    SOCKET_RELEASE_EVENT_SIZE,
};

pub const TLS_PAYLOAD_COMPLETION_EVENT_KIND: u32 = 201;
pub const TLS_PAYLOAD_CAPTURE_REQUEST_EVENT_KIND: u32 = 202;
pub const TLS_PAYLOAD_DIRECT_CAPTURE_EVENT_KIND: u32 = 203;
pub const TLS_PAYLOAD_DIAGNOSTIC_EVENT_KIND: u32 = 204;
const LAUNCH_BINDING_FAILURE_EVENT_KIND: u32 = 205;
pub const FILE_EVENT_OPEN: u32 = 300;
pub const FILE_EVENT_READ_SUMMARY: u32 = 308;
pub const STDIO_PAYLOAD_EVENT_KIND: u32 = 400;
pub const STDIO_PAYLOAD_COMPLETION_EVENT_KIND: u32 = 401;
pub const SOCKET_PAYLOAD_EVENT_KIND: u32 = 500;
pub const SOCKET_PAYLOAD_COMPLETION_EVENT_KIND: u32 = 501;

const NET_CONNECT_EVENT_KIND: u32 = 100;
const NET_ACCEPT_EVENT_KIND: u32 = 101;
const FD_IO_SEND_EVENT_KIND: u32 = 102;
const FD_IO_RECV_EVENT_KIND: u32 = 103;
const NET_BIND_EVENT_KIND: u32 = 104;
const NET_LISTEN_EVENT_KIND: u32 = 105;
const NET_CLOSE_EVENT_KIND: u32 = 106;
const NET_SHUTDOWN_EVENT_KIND: u32 = 107;
const SOCKET_FD_RELEASE_EVENT_KIND: u32 = 108;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelEvent {
    Observation(KernelObservationEvent),
    FilePath(KernelFilePathEvent),
    TlsCaptureRequest(KernelTlsCaptureRequestEvent),
    TlsCompletion(KernelTlsCompletionEvent),
    TlsDirectCapture(KernelTlsDirectCaptureEvent),
    TlsDiagnostic(KernelTlsDiagnosticEvent),
    LaunchBindingFailure(LaunchBindingFailure),
    StdioPayload(KernelStdioPayloadEvent),
    StdioPayloadCompletion(KernelStdioPayloadCompletionEvent),
    SocketPayload(KernelSocketPayloadEvent),
    SocketPayloadCompletion(KernelSocketPayloadCompletionEvent),
}

impl KernelEvent {
    pub(super) fn observed_ktime_ns(&self) -> Option<u64> {
        Some(match self {
            Self::Observation(event) => event.common.observed_ktime_ns,
            Self::FilePath(event) => event.observed_ktime_ns,
            Self::TlsCaptureRequest(event) => event.observed_ktime_ns,
            Self::TlsCompletion(event) => event.observed_ktime_ns,
            Self::TlsDirectCapture(event) => event.observed_ktime_ns,
            Self::TlsDiagnostic(event) => event.observed_ktime_ns,
            Self::LaunchBindingFailure(_) => return None,
            Self::StdioPayload(event) => event.observed_ktime_ns,
            Self::StdioPayloadCompletion(event) => event.observed_ktime_ns,
            Self::SocketPayload(event) => event.observed_ktime_ns,
            Self::SocketPayloadCompletion(event) => event.observed_ktime_ns,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchBindingFailure {
    trace_id: TraceId,
    status: LaunchBindingFailureStatus,
}

impl LaunchBindingFailure {
    pub const fn trace_id(self) -> TraceId {
        self.trace_id
    }

    pub const fn status(self) -> LaunchBindingFailureStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchBindingFailureStatus {
    IdentityFailure,
    PromotionFailure,
    CleanupFailure,
}

impl LaunchBindingFailureStatus {
    pub const fn code(self) -> &'static str {
        match self {
            Self::IdentityFailure => "launch_binding.identity_failure",
            Self::PromotionFailure => "launch_binding.promotion_failure",
            Self::CleanupFailure => "launch_binding.cleanup_failure",
        }
    }

    fn decode(raw: u32) -> Result<Self, LoaderError> {
        match raw {
            1 => Ok(Self::IdentityFailure),
            2 => Ok(Self::PromotionFailure),
            3 => Ok(Self::CleanupFailure),
            _ => Err(LoaderError::new(
                "decode_launch_binding_failure",
                format!("unknown launch binding failure status {raw}"),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelEndpoint {
    pub family: u16,
    pub port_be: u16,
    pub addr4_be: u32,
    pub addr6: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelObservationEvent {
    pub common: KernelObservationCommon,
    pub payload: KernelObservationPayload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelObservationCommon {
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub subject: KernelEventIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelEventIdentity {
    pub observer_namespace_tgid: u32,
    pub kernel_tgid: u32,
    pub start_boottime_ns: u64,
}

impl KernelEventIdentity {
    pub const fn binding_tgid(self) -> u32 {
        if self.kernel_tgid != 0 {
            self.kernel_tgid
        } else {
            self.observer_namespace_tgid
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelObservationPayload {
    Fork(KernelForkPayload),
    Exec(KernelExecPayload),
    Exit(KernelExitPayload),
    SignalGenerate(KernelSignalPayload),
    Network(KernelNetworkPayload),
    FdIo(KernelFdIoPayload),
    SocketFdRelease(KernelSocketFdReleasePayload),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelForkPayload {
    pub parent: KernelEventIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelExecPayload {
    pub filename: Option<KernelExecFilename>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelExitPayload {
    pub exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelSignalPayload {
    pub signal_result: i32,
    pub signal: u32,
    pub target_kernel_tid: u32,
    pub target_group: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelNetworkOperation {
    Connect,
    Accept,
    Bind,
    Listen,
    Close,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelFdIoOperation {
    Send,
    Recv,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelNetworkPayload {
    pub operation: KernelNetworkOperation,
    pub syscall_result: i32,
    pub fd: u32,
    pub syscall_family: u32,
    pub operation_flags: u32,
    pub fd_object_generation: u64,
    pub endpoint: Option<KernelEndpointWithRole>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelFdIoPayload {
    pub operation: KernelFdIoOperation,
    pub syscall_result: i32,
    pub fd: u32,
    pub syscall_family: u32,
    pub fd_category: u32,
    pub requested_size: u64,
    pub fd_object_generation: u64,
    pub endpoint: Option<KernelEndpointWithRole>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelSocketFdReleasePayload {
    pub fd: u32,
    pub fd_object_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelEndpointRole {
    Local,
    Remote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelEndpointWithRole {
    pub role: KernelEndpointRole,
    pub endpoint: KernelEndpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KernelTypedEventHeader {
    kind: u32,
    trace_id: TraceId,
    observed_ktime_ns: u64,
    subject_observer_namespace_tgid: u32,
    subject_kernel_tgid: u32,
    subject_start_boottime_ns: u64,
}

impl KernelTypedEventHeader {
    fn decode(raw: &[u8], expected_kind: u32, expected_size: usize) -> Result<Self, LoaderError> {
        if raw.len() != expected_size {
            return Err(LoaderError::new(
                "decode_typed_event_header",
                format!(
                    "unexpected event size {}, expected {expected_size} for kind {expected_kind}",
                    raw.len()
                ),
            ));
        }
        let kind = read_u32(raw, 0).expect("event length checked");
        let revision = read_u16(raw, 4).expect("event length checked");
        let record_size = usize::from(read_u16(raw, 6).expect("event length checked"));
        if kind != expected_kind || revision != EVENT_ABI_REVISION || record_size != expected_size {
            return Err(LoaderError::new(
                "decode_typed_event_header",
                format!(
                    "invalid typed event header kind={kind} revision={revision} record_size={record_size}; expected kind={expected_kind} revision={EVENT_ABI_REVISION} record_size={expected_size}"
                ),
            ));
        }
        Ok(Self {
            kind,
            trace_id: TraceId::new(read_u64(raw, 8).expect("event length checked")),
            observed_ktime_ns: read_u64(raw, 16).expect("event length checked"),
            subject_observer_namespace_tgid: read_u32(raw, 24).expect("event length checked"),
            subject_kernel_tgid: read_u32(raw, 28).expect("event length checked"),
            subject_start_boottime_ns: read_u64(raw, 32).expect("event length checked"),
        })
    }

    fn common(self) -> KernelObservationCommon {
        KernelObservationCommon {
            trace_id: self.trace_id,
            observed_ktime_ns: self.observed_ktime_ns,
            subject: KernelEventIdentity {
                observer_namespace_tgid: self.subject_observer_namespace_tgid,
                kernel_tgid: self.subject_kernel_tgid,
                start_boottime_ns: self.subject_start_boottime_ns,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelExecFilename {
    pub path: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelFilePathEvent {
    pub kind: u32,
    pub pid: u32,
    pub tid: u32,
    pub phase: u32,
    pub result: i64,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub fd: u32,
    pub aux: u32,
    pub path_size: u32,
    pub path_flags: u32,
    pub secondary_path_size: u32,
    pub secondary_path_flags: u32,
    pub path_max_bytes: u32,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
    pub arg5: u64,
    pub pid_generation: u64,
    pub path: Vec<u8>,
    pub secondary_path: Vec<u8>,
}

pub fn decode_kernel_event(raw: &[u8]) -> Result<KernelEvent, LoaderError> {
    let Some(kind) = read_u32(raw, 0) else {
        return Err(LoaderError::new(
            "decode_kernel_event",
            format!("unexpected empty kernel event size {}", raw.len()),
        ));
    };
    if kind == PROC_FORK_EVENT_KIND {
        return decode_process_fork_event(raw).map(KernelEvent::Observation);
    }
    if kind == PROC_EXEC_EVENT_KIND {
        return decode_process_exec_event(raw).map(KernelEvent::Observation);
    }
    if kind == PROC_EXIT_EVENT_KIND {
        return decode_process_exit_event(raw).map(KernelEvent::Observation);
    }
    if kind == PROC_SIGNAL_EVENT_KIND {
        return decode_process_signal_event(raw).map(KernelEvent::Observation);
    }
    if matches!(
        kind,
        NET_CONNECT_EVENT_KIND
            | NET_ACCEPT_EVENT_KIND
            | NET_BIND_EVENT_KIND
            | NET_LISTEN_EVENT_KIND
            | NET_CLOSE_EVENT_KIND
            | NET_SHUTDOWN_EVENT_KIND
    ) {
        return decode_network_event(raw, kind).map(KernelEvent::Observation);
    }
    if matches!(kind, FD_IO_SEND_EVENT_KIND | FD_IO_RECV_EVENT_KIND) {
        return decode_fd_io_event(raw, kind).map(KernelEvent::Observation);
    }
    if kind == SOCKET_FD_RELEASE_EVENT_KIND {
        return decode_socket_release_event(raw).map(KernelEvent::Observation);
    }
    if kind == TLS_PAYLOAD_COMPLETION_EVENT_KIND {
        return decode_tls_completion_event(raw).map(KernelEvent::TlsCompletion);
    }
    if kind == TLS_PAYLOAD_CAPTURE_REQUEST_EVENT_KIND {
        return decode_tls_capture_request_event(raw).map(KernelEvent::TlsCaptureRequest);
    }
    if kind == TLS_PAYLOAD_DIRECT_CAPTURE_EVENT_KIND {
        return decode_tls_direct_capture_event(raw).map(KernelEvent::TlsDirectCapture);
    }
    if kind == TLS_PAYLOAD_DIAGNOSTIC_EVENT_KIND {
        return decode_tls_diagnostic_event(raw).map(KernelEvent::TlsDiagnostic);
    }
    if kind == LAUNCH_BINDING_FAILURE_EVENT_KIND {
        return decode_launch_binding_failure(raw).map(KernelEvent::LaunchBindingFailure);
    }
    if kind == STDIO_PAYLOAD_EVENT_KIND {
        return decode_stdio_payload_event(raw).map(KernelEvent::StdioPayload);
    }
    if kind == STDIO_PAYLOAD_COMPLETION_EVENT_KIND {
        return decode_stdio_payload_completion_event(raw).map(KernelEvent::StdioPayloadCompletion);
    }
    if kind == SOCKET_PAYLOAD_EVENT_KIND {
        return decode_socket_payload_event(raw).map(KernelEvent::SocketPayload);
    }
    if kind == SOCKET_PAYLOAD_COMPLETION_EVENT_KIND {
        return decode_socket_payload_completion_event(raw)
            .map(KernelEvent::SocketPayloadCompletion);
    }
    if (FILE_EVENT_OPEN..=FILE_EVENT_READ_SUMMARY).contains(&kind) {
        return decode_file_path_event(raw).map(KernelEvent::FilePath);
    }
    Err(LoaderError::new(
        "decode_kernel_event",
        format!("unknown kernel event kind {kind}"),
    ))
}

fn decode_launch_binding_failure(raw: &[u8]) -> Result<LaunchBindingFailure, LoaderError> {
    if raw.len() != LAUNCH_BINDING_FAILURE_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_launch_binding_failure",
            format!(
                "unexpected launch binding failure event size {}, expected {}",
                raw.len(),
                LAUNCH_BINDING_FAILURE_EVENT_SIZE
            ),
        ));
    }
    let trace_id = TraceId::new(read_u64(raw, 8).expect("event length checked"));
    if trace_id.get() == 0 {
        return Err(LoaderError::new(
            "decode_launch_binding_failure",
            "launch binding failure event has zero trace ID",
        ));
    }
    Ok(LaunchBindingFailure {
        trace_id,
        status: LaunchBindingFailureStatus::decode(
            read_u32(raw, 4).expect("event length checked"),
        )?,
    })
}

fn decode_process_fork_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let header =
        KernelTypedEventHeader::decode(raw, PROC_FORK_EVENT_KIND, PROCESS_FORK_EVENT_SIZE)?;
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Fork(KernelForkPayload {
            parent: KernelEventIdentity {
                observer_namespace_tgid: read_u32(raw, EVENT_HEADER_SIZE)
                    .expect("event length checked"),
                kernel_tgid: read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked"),
                start_boottime_ns: read_u64(raw, EVENT_HEADER_SIZE + 8)
                    .expect("event length checked"),
            },
        }),
    })
}

fn decode_process_exec_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let header =
        KernelTypedEventHeader::decode(raw, PROC_EXEC_EVENT_KIND, PROCESS_EXEC_EVENT_SIZE)?;
    let mut filename = None;
    let filename_size =
        read_u32(raw, EXEC_EVENT_FILENAME_SIZE_OFFSET).expect("event length checked");
    let filename_size = usize::try_from(filename_size).map_err(|error| {
        LoaderError::new(
            "decode_exec_event",
            format!("filename size overflow: {error}"),
        )
    })?;
    if filename_size > EXEC_FILENAME_ABI_MAX_BYTES {
        return Err(LoaderError::new(
            "decode_exec_event",
            format!(
                "exec filename size {} exceeds ABI maximum {}",
                filename_size, EXEC_FILENAME_ABI_MAX_BYTES
            ),
        ));
    }
    let flags = read_u32(raw, EXEC_EVENT_FILENAME_FLAGS_OFFSET).expect("event length checked");
    if filename_size > 0 {
        let filename_end = EXEC_EVENT_FILENAME_OFFSET + filename_size;
        filename = Some(KernelExecFilename {
            path: String::from_utf8_lossy(&raw[EXEC_EVENT_FILENAME_OFFSET..filename_end])
                .into_owned(),
            truncated: flags & EXEC_FILENAME_FLAG_TRUNCATED != 0,
        });
    }
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Exec(KernelExecPayload { filename }),
    })
}

fn decode_process_exit_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    const EXIT_CODE_VALID: u32 = 1;
    let header =
        KernelTypedEventHeader::decode(raw, PROC_EXIT_EVENT_KIND, PROCESS_EXIT_EVENT_SIZE)?;
    let exit_flags = read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked");
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Exit(KernelExitPayload {
            exit_code: (exit_flags & EXIT_CODE_VALID != 0)
                .then(|| read_i32(raw, EVENT_HEADER_SIZE).expect("event length checked")),
        }),
    })
}

fn decode_process_signal_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let header =
        KernelTypedEventHeader::decode(raw, PROC_SIGNAL_EVENT_KIND, PROCESS_SIGNAL_EVENT_SIZE)?;
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::SignalGenerate(KernelSignalPayload {
            signal_result: read_i32(raw, EVENT_HEADER_SIZE).expect("event length checked"),
            signal: read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked"),
            target_kernel_tid: read_u32(raw, EVENT_HEADER_SIZE + 8).expect("event length checked"),
            target_group: read_u32(raw, EVENT_HEADER_SIZE + 12).expect("event length checked"),
        }),
    })
}

fn decode_network_event(raw: &[u8], kind: u32) -> Result<KernelObservationEvent, LoaderError> {
    const ENDPOINT_OFFSET: usize = EVENT_HEADER_SIZE + 28;
    let header = KernelTypedEventHeader::decode(raw, kind, NETWORK_EVENT_SIZE)?;
    let operation = match kind {
        NET_CONNECT_EVENT_KIND => KernelNetworkOperation::Connect,
        NET_ACCEPT_EVENT_KIND => KernelNetworkOperation::Accept,
        NET_BIND_EVENT_KIND => KernelNetworkOperation::Bind,
        NET_LISTEN_EVENT_KIND => KernelNetworkOperation::Listen,
        NET_CLOSE_EVENT_KIND => KernelNetworkOperation::Close,
        NET_SHUTDOWN_EVENT_KIND => KernelNetworkOperation::Shutdown,
        _ => unreachable!("network kind dispatched"),
    };
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::Network(KernelNetworkPayload {
            operation,
            syscall_result: read_i32(raw, EVENT_HEADER_SIZE).expect("event length checked"),
            fd: read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked"),
            syscall_family: read_u32(raw, EVENT_HEADER_SIZE + 8).expect("event length checked"),
            operation_flags: read_u32(raw, EVENT_HEADER_SIZE + 12).expect("event length checked"),
            fd_object_generation: read_u64(raw, EVENT_HEADER_SIZE + 16)
                .expect("event length checked"),
            endpoint: decode_typed_endpoint(
                read_u32(raw, EVENT_HEADER_SIZE + 24).expect("event length checked"),
                &raw[ENDPOINT_OFFSET..ENDPOINT_OFFSET + KERNEL_ENDPOINT_SIZE],
            )?,
        }),
    })
}

fn decode_fd_io_event(raw: &[u8], kind: u32) -> Result<KernelObservationEvent, LoaderError> {
    const ENDPOINT_OFFSET: usize = EVENT_HEADER_SIZE + 32;
    let header = KernelTypedEventHeader::decode(raw, kind, FD_IO_EVENT_SIZE)?;
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::FdIo(KernelFdIoPayload {
            operation: if kind == FD_IO_SEND_EVENT_KIND {
                KernelFdIoOperation::Send
            } else {
                KernelFdIoOperation::Recv
            },
            syscall_result: read_i32(raw, EVENT_HEADER_SIZE).expect("event length checked"),
            fd: read_u32(raw, EVENT_HEADER_SIZE + 4).expect("event length checked"),
            syscall_family: read_u32(raw, EVENT_HEADER_SIZE + 8).expect("event length checked"),
            fd_category: read_u32(raw, EVENT_HEADER_SIZE + 12).expect("event length checked"),
            requested_size: read_u64(raw, EVENT_HEADER_SIZE + 16).expect("event length checked"),
            fd_object_generation: read_u64(raw, EVENT_HEADER_SIZE + 24)
                .expect("event length checked"),
            endpoint: decode_typed_endpoint(
                read_u32(raw, EVENT_HEADER_SIZE + 32).expect("event length checked"),
                &raw[ENDPOINT_OFFSET + 4..ENDPOINT_OFFSET + 4 + KERNEL_ENDPOINT_SIZE],
            )?,
        }),
    })
}

fn decode_socket_release_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let header = KernelTypedEventHeader::decode(
        raw,
        SOCKET_FD_RELEASE_EVENT_KIND,
        SOCKET_RELEASE_EVENT_SIZE,
    )?;
    Ok(KernelObservationEvent {
        common: header.common(),
        payload: KernelObservationPayload::SocketFdRelease(KernelSocketFdReleasePayload {
            fd: read_u32(raw, EVENT_HEADER_SIZE).expect("event length checked"),
            fd_object_generation: read_u64(raw, EVENT_HEADER_SIZE + 4)
                .expect("event length checked"),
        }),
    })
}

fn decode_typed_endpoint(
    role: u32,
    raw: &[u8],
) -> Result<Option<KernelEndpointWithRole>, LoaderError> {
    let endpoint = decode_endpoint(raw)?;
    match role {
        0 => Ok(None),
        1 => Ok(Some(KernelEndpointWithRole {
            role: KernelEndpointRole::Local,
            endpoint,
        })),
        2 => Ok(Some(KernelEndpointWithRole {
            role: KernelEndpointRole::Remote,
            endpoint,
        })),
        _ => Err(LoaderError::new(
            "decode_typed_endpoint",
            format!("unknown endpoint role {role}"),
        )),
    }
}

fn decode_endpoint(raw: &[u8]) -> Result<KernelEndpoint, LoaderError> {
    const KERNEL_ENDPOINT_SIZE: usize = 24;
    if raw.len() != KERNEL_ENDPOINT_SIZE {
        return Err(LoaderError::new(
            "decode_endpoint",
            format!("unexpected endpoint size {}", raw.len()),
        ));
    }

    Ok(KernelEndpoint {
        family: u16::from_ne_bytes(raw[0..2].try_into().expect("slice length checked")),
        port_be: u16::from_ne_bytes(raw[2..4].try_into().expect("slice length checked")),
        addr4_be: u32::from_ne_bytes(raw[4..8].try_into().expect("slice length checked")),
        addr6: raw[8..24].try_into().expect("slice length checked"),
    })
}

fn read_u32(raw: &[u8], offset: usize) -> Option<u32> {
    raw.get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_ne_bytes)
}

fn read_u16(raw: &[u8], offset: usize) -> Option<u16> {
    raw.get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_ne_bytes)
}

fn read_i32(raw: &[u8], offset: usize) -> Option<i32> {
    raw.get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(i32::from_ne_bytes)
}

fn read_i64(raw: &[u8], offset: usize) -> Option<i64> {
    raw.get(offset..offset + 8)
        .and_then(|value| value.try_into().ok())
        .map(i64::from_ne_bytes)
}

fn read_u64(raw: &[u8], offset: usize) -> Option<u64> {
    raw.get(offset..offset + 8)
        .and_then(|value| value.try_into().ok())
        .map(u64::from_ne_bytes)
}

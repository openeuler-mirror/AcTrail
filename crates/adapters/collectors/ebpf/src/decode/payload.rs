//! Payload kernel-event decoding.

use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use payload_event::RawPayloadSegment;

use crate::decode::{DecodeError, resolve_event_identity};
use crate::loader::{
    KernelSocketPayloadCompletionEvent, KernelSocketPayloadEvent, KernelStdioPayloadEvent,
    KernelTlsCaptureRequestEvent, KernelTlsCompletionEvent, KernelTlsDiagnosticEvent,
    KernelTlsDirectCaptureEvent,
};
use crate::maps::BindingStateMap;
use crate::procfs::ProcfsIdentityReader;

const STDIO_PAYLOAD_DIRECTION_INBOUND: u32 = 1;
const STDIO_PAYLOAD_DIRECTION_OUTBOUND: u32 = 2;
const STDIO_PAYLOAD_STREAM_STDIN: u32 = 0;
const STDIO_PAYLOAD_STREAM_STDOUT: u32 = 1;
const STDIO_PAYLOAD_STREAM_STDERR: u32 = 2;
const STDIO_PAYLOAD_SYSCALL_READ: u32 = 1;
const STDIO_PAYLOAD_SYSCALL_WRITE: u32 = 2;
const STDIO_PAYLOAD_FLAG_TRUNCATED: u32 = 1;
pub const SOCKET_PAYLOAD_DIRECTION_INBOUND: u32 = 1;
pub const SOCKET_PAYLOAD_DIRECTION_OUTBOUND: u32 = 2;
pub const SOCKET_PAYLOAD_SYSCALL_READ: u32 = 1;
pub const SOCKET_PAYLOAD_SYSCALL_WRITE: u32 = 2;
pub const SOCKET_PAYLOAD_SYSCALL_SENDTO: u32 = 3;
pub const SOCKET_PAYLOAD_SYSCALL_RECVFROM: u32 = 4;
pub const SOCKET_PAYLOAD_SYSCALL_WRITEV: u32 = 5;
pub const SOCKET_PAYLOAD_SYSCALL_SENDMSG: u32 = 6;
const SOCKET_PAYLOAD_FLAG_TRUNCATED: u32 = 1;
const TLS_PAYLOAD_COMPLETION_FAILED: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsPayloadCompletion {
    pub pid: u32,
    pub tid: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub operation_id: u64,
    pub stream_key: u64,
    pub direction: u32,
    pub symbol: u32,
    pub library: u32,
    pub pid_generation: u64,
    pub completed_size: u64,
    pub failed: bool,
    pub buffer_ptr: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsPayloadCaptureRequest {
    pub pid: u32,
    pub tid: u32,
    pub trace_id: TraceId,
    pub operation_id: u64,
    pub stream_key: u64,
    pub direction: u32,
    pub symbol: u32,
    pub library: u32,
    pub pid_generation: u64,
    pub requested_size: u64,
    pub buffer_ptr: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsPayloadDirectCapture {
    pub pid: u32,
    pub tid: u32,
    pub trace_id: TraceId,
    pub operation_id: u64,
    pub stream_key: u64,
    pub direction: u32,
    pub symbol: u32,
    pub library: u32,
    pub pid_generation: u64,
    pub original_size: u64,
    pub captured_size: u64,
    pub flags: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsDiagnosticEvent {
    pub reason: u32,
    pub host_tgid: u32,
    pub host_tid: u32,
    pub namespace_tgid: u32,
    pub namespace_tid: u32,
    pub direction: u32,
    pub symbol: u32,
    pub library: u32,
    pub lookup_flags: u32,
    pub requested_size: u64,
    pub buffer_ptr: u64,
    pub comm: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocketPayloadCompletion {
    pub pid: u32,
    pub tid: u32,
    pub trace_id: TraceId,
    pub sequence: u64,
    pub direction: u32,
    pub fd: u32,
    pub syscall: u32,
    pub fd_generation: u32,
    pub pid_generation: u64,
    pub completed_size: u64,
    pub requested_size: u64,
    pub buffer_ptr: u64,
}

pub fn decode_stdio_payload(
    event: KernelStdioPayloadEvent,
    bindings: &BindingStateMap,
    identity_reader: &ProcfsIdentityReader,
) -> Result<RawPayloadSegment, DecodeError> {
    let identity = resolve_payload_identity(
        event.trace_id,
        event.pid,
        event.pid_generation,
        bindings,
        identity_reader,
    )?;
    let stream = stdio_stream(event.stream)?;

    Ok(RawPayloadSegment {
        trace_id: event.trace_id,
        observed_at: SystemTime::now(),
        process: identity,
        source_boundary: PayloadSourceBoundary::Stdio,
        content_state: PayloadContentState::Plaintext,
        direction: stdio_direction(event.direction)?,
        stream_key: PayloadStreamKey::new(format!("stdio:{}:{stream}", event.pid)),
        sequence: event.sequence,
        original_size: u64::from(event.original_size),
        captured_size: u64::from(event.captured_size),
        operation_id: event.sequence,
        operation_offset: 0,
        operation_original_size: u64::from(event.original_size),
        operation_captured_size: u64::from(event.captured_size),
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: stdio_truncation(event.flags),
        library: "stdio".to_string(),
        symbol: stdio_symbol(event.syscall)?.to_string(),
        protocol_hint: Some(stream.to_string()),
        bytes: event.bytes,
    })
}

pub fn decode_socket_payload(
    event: KernelSocketPayloadEvent,
    bindings: &BindingStateMap,
    identity_reader: &ProcfsIdentityReader,
) -> Result<RawPayloadSegment, DecodeError> {
    let identity = resolve_payload_identity(
        event.trace_id,
        event.pid,
        event.pid_generation,
        bindings,
        identity_reader,
    )?;

    Ok(RawPayloadSegment {
        trace_id: event.trace_id,
        observed_at: SystemTime::now(),
        process: identity,
        source_boundary: PayloadSourceBoundary::Syscall,
        content_state: PayloadContentState::Plaintext,
        direction: socket_direction(event.direction)?,
        stream_key: PayloadStreamKey::new(format!(
            "socket:{}:{}:{}",
            event.pid, event.fd, event.fd_generation
        )),
        sequence: event.sequence,
        original_size: u64::from(event.original_size),
        captured_size: u64::from(event.captured_size),
        operation_id: event.sequence,
        operation_offset: 0,
        operation_original_size: u64::from(event.original_size),
        operation_captured_size: u64::from(event.captured_size),
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: socket_truncation(event.flags),
        library: "socket-syscall".to_string(),
        symbol: socket_symbol(event.syscall)?.to_string(),
        protocol_hint: None,
        bytes: event.bytes,
    })
}

pub fn decode_socket_payload_completion(
    event: KernelSocketPayloadCompletionEvent,
) -> SocketPayloadCompletion {
    SocketPayloadCompletion {
        pid: event.pid,
        tid: event.tid,
        trace_id: event.trace_id,
        sequence: event.sequence,
        direction: event.direction,
        fd: event.fd,
        syscall: event.syscall,
        fd_generation: event.fd_generation,
        pid_generation: event.pid_generation,
        completed_size: event.completed_size,
        requested_size: event.requested_size,
        buffer_ptr: event.buffer_ptr,
    }
}

pub fn decode_tls_capture_request(event: KernelTlsCaptureRequestEvent) -> TlsPayloadCaptureRequest {
    TlsPayloadCaptureRequest {
        pid: event.pid,
        tid: event.tid,
        trace_id: event.trace_id,
        operation_id: event.operation_id,
        stream_key: event.stream_key,
        direction: event.direction,
        symbol: event.symbol,
        library: event.library,
        pid_generation: event.pid_generation,
        requested_size: event.requested_size,
        buffer_ptr: event.buffer_ptr,
    }
}

pub fn decode_tls_completion(event: KernelTlsCompletionEvent) -> TlsPayloadCompletion {
    TlsPayloadCompletion {
        pid: event.pid,
        tid: event.tid,
        trace_id: event.trace_id,
        observed_ktime_ns: event.observed_ktime_ns,
        operation_id: event.operation_id,
        stream_key: event.stream_key,
        direction: event.direction,
        symbol: event.symbol,
        library: event.library,
        pid_generation: event.pid_generation,
        completed_size: u64::from(event.completed_size),
        failed: event.flags & TLS_PAYLOAD_COMPLETION_FAILED == TLS_PAYLOAD_COMPLETION_FAILED,
        buffer_ptr: event.buffer_ptr,
    }
}

pub fn decode_tls_direct_capture(event: KernelTlsDirectCaptureEvent) -> TlsPayloadDirectCapture {
    TlsPayloadDirectCapture {
        pid: event.pid,
        tid: event.tid,
        trace_id: event.trace_id,
        operation_id: event.operation_id,
        stream_key: event.stream_key,
        direction: event.direction,
        symbol: event.symbol,
        library: event.library,
        pid_generation: event.pid_generation,
        original_size: u64::from(event.original_size),
        captured_size: u64::from(event.captured_size),
        flags: event.flags,
        bytes: event.bytes,
    }
}

pub fn decode_tls_diagnostic(event: KernelTlsDiagnosticEvent) -> TlsDiagnosticEvent {
    TlsDiagnosticEvent {
        reason: event.reason,
        host_tgid: event.host_tgid,
        host_tid: event.host_tid,
        namespace_tgid: event.namespace_tgid,
        namespace_tid: event.namespace_tid,
        direction: event.direction,
        symbol: event.symbol,
        library: event.library,
        lookup_flags: event.lookup_flags,
        requested_size: event.requested_size,
        buffer_ptr: event.buffer_ptr,
        comm: kernel_comm(&event.comm),
    }
}

fn resolve_payload_identity(
    trace_id: TraceId,
    pid: u32,
    generation: u64,
    bindings: &BindingStateMap,
    identity_reader: &ProcfsIdentityReader,
) -> Result<model_core::process::ProcessIdentity, DecodeError> {
    resolve_event_identity(trace_id, pid, generation, bindings, identity_reader)
        .map_err(|error| DecodeError::new("payload_identity", error))
}

fn stdio_direction(raw: u32) -> Result<PayloadDirection, DecodeError> {
    match raw {
        STDIO_PAYLOAD_DIRECTION_OUTBOUND => Ok(PayloadDirection::Outbound),
        STDIO_PAYLOAD_DIRECTION_INBOUND => Ok(PayloadDirection::Inbound),
        other => Err(DecodeError::new(
            "stdio_payload_direction",
            format!("unknown stdio payload direction {other}"),
        )),
    }
}

fn socket_direction(raw: u32) -> Result<PayloadDirection, DecodeError> {
    match raw {
        SOCKET_PAYLOAD_DIRECTION_OUTBOUND => Ok(PayloadDirection::Outbound),
        SOCKET_PAYLOAD_DIRECTION_INBOUND => Ok(PayloadDirection::Inbound),
        other => Err(DecodeError::new(
            "socket_payload_direction",
            format!("unknown socket payload direction {other}"),
        )),
    }
}

fn stdio_stream(raw: u32) -> Result<&'static str, DecodeError> {
    match raw {
        STDIO_PAYLOAD_STREAM_STDIN => Ok("stdin"),
        STDIO_PAYLOAD_STREAM_STDOUT => Ok("stdout"),
        STDIO_PAYLOAD_STREAM_STDERR => Ok("stderr"),
        other => Err(DecodeError::new(
            "stdio_payload_stream",
            format!("unknown stdio payload stream {other}"),
        )),
    }
}

fn stdio_symbol(raw: u32) -> Result<&'static str, DecodeError> {
    match raw {
        STDIO_PAYLOAD_SYSCALL_READ => Ok("read"),
        STDIO_PAYLOAD_SYSCALL_WRITE => Ok("write"),
        other => Err(DecodeError::new(
            "stdio_payload_syscall",
            format!("unknown stdio payload syscall {other}"),
        )),
    }
}

fn socket_symbol(raw: u32) -> Result<&'static str, DecodeError> {
    match raw {
        SOCKET_PAYLOAD_SYSCALL_READ => Ok("read"),
        SOCKET_PAYLOAD_SYSCALL_WRITE => Ok("write"),
        SOCKET_PAYLOAD_SYSCALL_SENDTO => Ok("sendto"),
        SOCKET_PAYLOAD_SYSCALL_RECVFROM => Ok("recvfrom"),
        SOCKET_PAYLOAD_SYSCALL_WRITEV => Ok("writev"),
        SOCKET_PAYLOAD_SYSCALL_SENDMSG => Ok("sendmsg"),
        other => Err(DecodeError::new(
            "socket_payload_syscall",
            format!("unknown socket payload syscall {other}"),
        )),
    }
}

fn kernel_comm(raw: &[u8; 16]) -> String {
    let len = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..len]).to_string()
}

fn stdio_truncation(flags: u32) -> PayloadTruncationState {
    if flags & STDIO_PAYLOAD_FLAG_TRUNCATED == STDIO_PAYLOAD_FLAG_TRUNCATED {
        PayloadTruncationState::Truncated
    } else {
        PayloadTruncationState::Complete
    }
}

fn socket_truncation(flags: u32) -> PayloadTruncationState {
    if flags & SOCKET_PAYLOAD_FLAG_TRUNCATED == SOCKET_PAYLOAD_FLAG_TRUNCATED {
        PayloadTruncationState::Truncated
    } else {
        PayloadTruncationState::Complete
    }
}

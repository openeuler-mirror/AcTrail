use model_core::ids::TraceId;

use crate::loader::LoaderError;

use super::{read_u32, read_u64};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelTlsCompletionEvent {
    pub pid: u32,
    pub tid: u32,
    pub direction: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub stream_key: u64,
    pub operation_id: u64,
    pub completed_size: u32,
    pub flags: u32,
    pub symbol: u32,
    pub library: u32,
    pub pid_generation: u64,
    pub buffer_ptr: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelTlsCaptureRequestEvent {
    pub pid: u32,
    pub tid: u32,
    pub direction: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub stream_key: u64,
    pub operation_id: u64,
    pub requested_size: u64,
    pub buffer_ptr: u64,
    pub pid_generation: u64,
    pub symbol: u32,
    pub library: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelTlsDirectCaptureEvent {
    pub pid: u32,
    pub tid: u32,
    pub direction: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub stream_key: u64,
    pub operation_id: u64,
    pub original_size: u32,
    pub captured_size: u32,
    pub flags: u32,
    pub symbol: u32,
    pub library: u32,
    pub pid_generation: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelTlsDiagnosticEvent {
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
    pub observed_ktime_ns: u64,
    pub comm: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelStdioPayloadEvent {
    pub pid: u32,
    pub tid: u32,
    pub direction: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub sequence: u64,
    pub stream: u32,
    pub original_size: u32,
    pub captured_size: u32,
    pub flags: u32,
    pub fd: u32,
    pub syscall: u32,
    pub pid_generation: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelSocketPayloadEvent {
    pub pid: u32,
    pub tid: u32,
    pub direction: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub sequence: u64,
    pub fd: u32,
    pub original_size: u32,
    pub captured_size: u32,
    pub flags: u32,
    pub syscall: u32,
    pub fd_generation: u32,
    pub pid_generation: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelSocketPayloadCompletionEvent {
    pub pid: u32,
    pub tid: u32,
    pub direction: u32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub sequence: u64,
    pub completed_size: u64,
    pub requested_size: u64,
    pub buffer_ptr: u64,
    pub pid_generation: u64,
    pub fd: u32,
    pub flags: u32,
    pub syscall: u32,
    pub fd_generation: u32,
}

pub(super) fn decode_tls_capture_request_event(
    raw: &[u8],
) -> Result<KernelTlsCaptureRequestEvent, LoaderError> {
    const TLS_CAPTURE_REQUEST_EVENT_SIZE: usize = 80;
    if raw.len() != TLS_CAPTURE_REQUEST_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_tls_capture_request",
            format!(
                "unexpected TLS capture request event size {}, expected {}",
                raw.len(),
                TLS_CAPTURE_REQUEST_EVENT_SIZE
            ),
        ));
    }
    Ok(KernelTlsCaptureRequestEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        direction: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        stream_key: read_u64(raw, 32).expect("event length checked"),
        operation_id: read_u64(raw, 40).expect("event length checked"),
        requested_size: read_u64(raw, 48).expect("event length checked"),
        buffer_ptr: read_u64(raw, 56).expect("event length checked"),
        pid_generation: read_u64(raw, 64).expect("event length checked"),
        symbol: read_u32(raw, 72).expect("event length checked"),
        library: read_u32(raw, 76).expect("event length checked"),
    })
}

pub(super) fn decode_tls_completion_event(
    raw: &[u8],
) -> Result<KernelTlsCompletionEvent, LoaderError> {
    const TLS_COMPLETION_EVENT_SIZE: usize = 80;
    if raw.len() != TLS_COMPLETION_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_tls_completion",
            format!(
                "unexpected TLS completion event size {}, expected {}",
                raw.len(),
                TLS_COMPLETION_EVENT_SIZE
            ),
        ));
    }
    Ok(KernelTlsCompletionEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        direction: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        stream_key: read_u64(raw, 32).expect("event length checked"),
        operation_id: read_u64(raw, 40).expect("event length checked"),
        completed_size: read_u32(raw, 48).expect("event length checked"),
        flags: read_u32(raw, 52).expect("event length checked"),
        symbol: read_u32(raw, 56).expect("event length checked"),
        library: read_u32(raw, 60).expect("event length checked"),
        pid_generation: read_u64(raw, 64).expect("event length checked"),
        buffer_ptr: read_u64(raw, 72).expect("event length checked"),
    })
}

pub(super) fn decode_tls_direct_capture_event(
    raw: &[u8],
) -> Result<KernelTlsDirectCaptureEvent, LoaderError> {
    const TLS_DIRECT_CAPTURE_HEADER_SIZE: usize = 80;
    const TLS_DIRECT_CAPTURE_ABI_MAX_BYTES: usize = 4_194_304;
    const TLS_DIRECT_CAPTURE_EVENT_SIZE: usize =
        TLS_DIRECT_CAPTURE_HEADER_SIZE + TLS_DIRECT_CAPTURE_ABI_MAX_BYTES;
    if raw.len() != TLS_DIRECT_CAPTURE_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_tls_direct_capture",
            format!(
                "unexpected TLS direct capture event size {}, expected {}",
                raw.len(),
                TLS_DIRECT_CAPTURE_EVENT_SIZE
            ),
        ));
    }
    let captured_size = read_u32(raw, 52).expect("event length checked");
    if captured_size as usize > TLS_DIRECT_CAPTURE_ABI_MAX_BYTES {
        return Err(LoaderError::new(
            "decode_tls_direct_capture",
            format!("TLS captured size {} exceeds ABI maximum", captured_size),
        ));
    }
    Ok(KernelTlsDirectCaptureEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        direction: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        stream_key: read_u64(raw, 32).expect("event length checked"),
        operation_id: read_u64(raw, 40).expect("event length checked"),
        original_size: read_u32(raw, 48).expect("event length checked"),
        captured_size,
        flags: read_u32(raw, 56).expect("event length checked"),
        symbol: read_u32(raw, 60).expect("event length checked"),
        library: read_u32(raw, 64).expect("event length checked"),
        pid_generation: read_u64(raw, 72).expect("event length checked"),
        bytes: raw[TLS_DIRECT_CAPTURE_HEADER_SIZE
            ..TLS_DIRECT_CAPTURE_HEADER_SIZE + captured_size as usize]
            .to_vec(),
    })
}

pub(super) fn decode_tls_diagnostic_event(
    raw: &[u8],
) -> Result<KernelTlsDiagnosticEvent, LoaderError> {
    const TLS_DIAGNOSTIC_EVENT_SIZE: usize = 80;
    const TLS_DIAGNOSTIC_COMM_OFFSET: usize = 64;
    if raw.len() != TLS_DIAGNOSTIC_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_tls_diagnostic",
            format!(
                "unexpected TLS diagnostic event size {}, expected {}",
                raw.len(),
                TLS_DIAGNOSTIC_EVENT_SIZE
            ),
        ));
    }
    let mut comm = [0; 16];
    comm.copy_from_slice(&raw[TLS_DIAGNOSTIC_COMM_OFFSET..TLS_DIAGNOSTIC_EVENT_SIZE]);
    Ok(KernelTlsDiagnosticEvent {
        reason: read_u32(raw, 4).expect("event length checked"),
        host_tgid: read_u32(raw, 8).expect("event length checked"),
        host_tid: read_u32(raw, 12).expect("event length checked"),
        namespace_tgid: read_u32(raw, 16).expect("event length checked"),
        namespace_tid: read_u32(raw, 20).expect("event length checked"),
        direction: read_u32(raw, 24).expect("event length checked"),
        symbol: read_u32(raw, 28).expect("event length checked"),
        library: read_u32(raw, 32).expect("event length checked"),
        lookup_flags: read_u32(raw, 36).expect("event length checked"),
        requested_size: read_u64(raw, 40).expect("event length checked"),
        buffer_ptr: read_u64(raw, 48).expect("event length checked"),
        observed_ktime_ns: read_u64(raw, 56).expect("event length checked"),
        comm,
    })
}

pub(super) fn decode_stdio_payload_event(
    raw: &[u8],
) -> Result<KernelStdioPayloadEvent, LoaderError> {
    const STDIO_EVENT_HEADER_SIZE: usize = 72;
    const STDIO_PAYLOAD_ABI_MAX_BYTES: usize = 4096;
    const STDIO_EVENT_SIZE: usize = STDIO_EVENT_HEADER_SIZE + STDIO_PAYLOAD_ABI_MAX_BYTES;
    if raw.len() != STDIO_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_stdio_payload",
            format!(
                "unexpected stdio payload event size {}, expected {}",
                raw.len(),
                STDIO_EVENT_SIZE
            ),
        ));
    }
    let captured_size = read_u32(raw, 48).expect("event length checked");
    if captured_size as usize > STDIO_PAYLOAD_ABI_MAX_BYTES {
        return Err(LoaderError::new(
            "decode_stdio_payload",
            format!("stdio captured size {} exceeds ABI maximum", captured_size),
        ));
    }
    Ok(KernelStdioPayloadEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        direction: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        sequence: read_u64(raw, 32).expect("event length checked"),
        stream: read_u32(raw, 40).expect("event length checked"),
        original_size: read_u32(raw, 44).expect("event length checked"),
        captured_size,
        flags: read_u32(raw, 52).expect("event length checked"),
        fd: read_u32(raw, 56).expect("event length checked"),
        syscall: read_u32(raw, 60).expect("event length checked"),
        pid_generation: read_u64(raw, 64).expect("event length checked"),
        bytes: raw[STDIO_EVENT_HEADER_SIZE..STDIO_EVENT_HEADER_SIZE + captured_size as usize]
            .to_vec(),
    })
}

pub(super) fn decode_socket_payload_event(
    raw: &[u8],
) -> Result<KernelSocketPayloadEvent, LoaderError> {
    const SOCKET_EVENT_HEADER_SIZE: usize = 72;
    const SOCKET_PAYLOAD_ABI_MAX_BYTES: usize = 4_096;
    const SOCKET_EVENT_SIZE: usize = SOCKET_EVENT_HEADER_SIZE + SOCKET_PAYLOAD_ABI_MAX_BYTES;
    if raw.len() != SOCKET_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_socket_payload",
            format!(
                "unexpected socket payload event size {}, expected {}",
                raw.len(),
                SOCKET_EVENT_SIZE
            ),
        ));
    }
    let captured_size = read_u32(raw, 48).expect("event length checked");
    if captured_size as usize > SOCKET_PAYLOAD_ABI_MAX_BYTES {
        return Err(LoaderError::new(
            "decode_socket_payload",
            format!("socket captured size {} exceeds ABI maximum", captured_size),
        ));
    }
    Ok(KernelSocketPayloadEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        direction: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        sequence: read_u64(raw, 32).expect("event length checked"),
        fd: read_u32(raw, 40).expect("event length checked"),
        original_size: read_u32(raw, 44).expect("event length checked"),
        captured_size,
        flags: read_u32(raw, 52).expect("event length checked"),
        syscall: read_u32(raw, 56).expect("event length checked"),
        fd_generation: read_u32(raw, 60).expect("event length checked"),
        pid_generation: read_u64(raw, 64).expect("event length checked"),
        bytes: raw[SOCKET_EVENT_HEADER_SIZE..SOCKET_EVENT_HEADER_SIZE + captured_size as usize]
            .to_vec(),
    })
}

pub(super) fn decode_socket_payload_completion_event(
    raw: &[u8],
) -> Result<KernelSocketPayloadCompletionEvent, LoaderError> {
    const SOCKET_COMPLETION_EVENT_SIZE: usize = 88;
    if raw.len() != SOCKET_COMPLETION_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_socket_payload_completion",
            format!(
                "unexpected socket payload completion event size {}, expected {}",
                raw.len(),
                SOCKET_COMPLETION_EVENT_SIZE
            ),
        ));
    }
    Ok(KernelSocketPayloadCompletionEvent {
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        direction: read_u32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        sequence: read_u64(raw, 32).expect("event length checked"),
        completed_size: read_u64(raw, 40).expect("event length checked"),
        requested_size: read_u64(raw, 48).expect("event length checked"),
        buffer_ptr: read_u64(raw, 56).expect("event length checked"),
        pid_generation: read_u64(raw, 64).expect("event length checked"),
        fd: read_u32(raw, 72).expect("event length checked"),
        flags: read_u32(raw, 76).expect("event length checked"),
        syscall: read_u32(raw, 80).expect("event length checked"),
        fd_generation: read_u32(raw, 84).expect("event length checked"),
    })
}

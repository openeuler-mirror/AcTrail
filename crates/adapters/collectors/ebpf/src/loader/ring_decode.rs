//! Ring-buffer record decoding.

#[path = "ring_decode/payload.rs"]
mod payload;

use model_core::ids::TraceId;

use crate::loader::LoaderError;

pub use payload::{
    KernelSocketPayloadCompletionEvent, KernelSocketPayloadEvent, KernelStdioPayloadEvent,
    KernelTlsCaptureRequestEvent, KernelTlsCompletionEvent, KernelTlsDiagnosticEvent,
    KernelTlsDirectCaptureEvent,
};
use payload::{
    decode_socket_payload_completion_event, decode_socket_payload_event,
    decode_stdio_payload_event, decode_tls_capture_request_event, decode_tls_completion_event,
    decode_tls_diagnostic_event, decode_tls_direct_capture_event,
};

use super::abi::{
    EXEC_EVENT_FILENAME_FLAGS_OFFSET, EXEC_EVENT_FILENAME_OFFSET, EXEC_EVENT_FILENAME_SIZE_OFFSET,
    EXEC_EVENT_SIZE, EXEC_FILENAME_ABI_MAX_BYTES, EXEC_FILENAME_FLAG_TRUNCATED,
    KERNEL_ENDPOINT_SIZE, KERNEL_OBSERVATION_EVENT_SIZE, KERNEL_OBSERVATION_HEADER_SIZE,
    PROC_EXEC_EVENT_KIND,
};

pub const TLS_PAYLOAD_COMPLETION_EVENT_KIND: u32 = 201;
pub const TLS_PAYLOAD_CAPTURE_REQUEST_EVENT_KIND: u32 = 202;
pub const TLS_PAYLOAD_DIRECT_CAPTURE_EVENT_KIND: u32 = 203;
pub const TLS_PAYLOAD_DIAGNOSTIC_EVENT_KIND: u32 = 204;
pub const FILE_EVENT_OPEN: u32 = 300;
pub const FILE_EVENT_READ_SUMMARY: u32 = 308;
pub const STDIO_PAYLOAD_EVENT_KIND: u32 = 400;
pub const SOCKET_PAYLOAD_EVENT_KIND: u32 = 500;
pub const SOCKET_PAYLOAD_COMPLETION_EVENT_KIND: u32 = 501;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelEvent {
    Observation(KernelObservationEvent),
    FilePath(KernelFilePathEvent),
    TlsCaptureRequest(KernelTlsCaptureRequestEvent),
    TlsCompletion(KernelTlsCompletionEvent),
    TlsDirectCapture(KernelTlsDirectCaptureEvent),
    TlsDiagnostic(KernelTlsDiagnosticEvent),
    StdioPayload(KernelStdioPayloadEvent),
    SocketPayload(KernelSocketPayloadEvent),
    SocketPayloadCompletion(KernelSocketPayloadCompletionEvent),
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
    pub kind: u32,
    pub pid: u32,
    pub aux: u32,
    pub result: i32,
    pub trace_id: TraceId,
    pub observed_ktime_ns: u64,
    pub fd: u32,
    pub reserved: u32,
    pub requested_size: u64,
    pub pid_generation: u64,
    pub aux_generation: u64,
    pub local: KernelEndpoint,
    pub remote: KernelEndpoint,
    pub exec_filename: Option<KernelExecFilename>,
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
            "decode_ring_buffer",
            format!("unexpected empty kernel event size {}", raw.len()),
        ));
    };
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
    if kind == STDIO_PAYLOAD_EVENT_KIND {
        return decode_stdio_payload_event(raw).map(KernelEvent::StdioPayload);
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
    decode_observation_event(raw).map(KernelEvent::Observation)
}

fn decode_observation_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let kind = read_u32(raw, 0).expect("event length checked");
    if kind == PROC_EXEC_EVENT_KIND && raw.len() == EXEC_EVENT_SIZE {
        return decode_exec_observation_event(raw);
    }
    if raw.len() != KERNEL_OBSERVATION_EVENT_SIZE {
        return Err(LoaderError::new(
            "decode_ring_buffer",
            format!(
                "unexpected kernel event size {}, expected {}",
                raw.len(),
                KERNEL_OBSERVATION_EVENT_SIZE
            ),
        ));
    }

    let local_endpoint_start = KERNEL_OBSERVATION_HEADER_SIZE;
    let remote_endpoint_start = local_endpoint_start + KERNEL_ENDPOINT_SIZE;
    Ok(KernelObservationEvent {
        kind,
        pid: read_u32(raw, 4).expect("event length checked"),
        aux: read_u32(raw, 8).expect("event length checked"),
        result: read_i32(raw, 12).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 16).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 24).expect("event length checked"),
        fd: read_u32(raw, 32).expect("event length checked"),
        reserved: read_u32(raw, 36).expect("event length checked"),
        requested_size: read_u64(raw, 40).expect("event length checked"),
        pid_generation: read_u64(raw, 48).expect("event length checked"),
        aux_generation: read_u64(raw, 56).expect("event length checked"),
        local: decode_endpoint(&raw[local_endpoint_start..remote_endpoint_start])?,
        remote: decode_endpoint(&raw[remote_endpoint_start..KERNEL_OBSERVATION_EVENT_SIZE])?,
        exec_filename: None,
    })
}

fn decode_exec_observation_event(raw: &[u8]) -> Result<KernelObservationEvent, LoaderError> {
    let mut event = decode_observation_event(&raw[..KERNEL_OBSERVATION_EVENT_SIZE])?;
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
        event.exec_filename = Some(KernelExecFilename {
            path: String::from_utf8_lossy(&raw[EXEC_EVENT_FILENAME_OFFSET..filename_end])
                .into_owned(),
            truncated: flags & EXEC_FILENAME_FLAG_TRUNCATED != 0,
        });
    }
    Ok(event)
}

fn decode_file_path_event(raw: &[u8]) -> Result<KernelFilePathEvent, LoaderError> {
    const FILE_PATH_ABI_MAX_BYTES: usize = 256;
    const FILE_EVENT_HEADER_SIZE: usize = 128;
    const FILE_EVENT_PRIMARY_PATH_SIZE: usize = FILE_EVENT_HEADER_SIZE + FILE_PATH_ABI_MAX_BYTES;
    const FILE_EVENT_SIZE: usize = FILE_EVENT_HEADER_SIZE + FILE_PATH_ABI_MAX_BYTES * 2;
    let compact = raw.len() == FILE_EVENT_HEADER_SIZE;
    let primary_path_only = raw.len() == FILE_EVENT_PRIMARY_PATH_SIZE;
    if raw.len() != FILE_EVENT_SIZE && !compact && !primary_path_only {
        return Err(LoaderError::new(
            "decode_file_path",
            format!(
                "unexpected file path event size {}, expected {}, {}, or {}",
                raw.len(),
                FILE_EVENT_HEADER_SIZE,
                FILE_EVENT_PRIMARY_PATH_SIZE,
                FILE_EVENT_SIZE
            ),
        ));
    }
    let path_size = read_u32(raw, 48).expect("event length checked");
    let secondary_path_size = read_u32(raw, 56).expect("event length checked");
    if compact && (path_size != 0 || secondary_path_size != 0) {
        return Err(LoaderError::new(
            "decode_file_path",
            format!(
                "compact file event carried path sizes path={} secondary={}",
                path_size, secondary_path_size
            ),
        ));
    }
    if primary_path_only && secondary_path_size != 0 {
        return Err(LoaderError::new(
            "decode_file_path",
            format!("primary-path file event carried secondary path size {secondary_path_size}"),
        ));
    }
    validate_path_size("path", path_size, FILE_PATH_ABI_MAX_BYTES)?;
    validate_path_size(
        "secondary_path",
        secondary_path_size,
        FILE_PATH_ABI_MAX_BYTES,
    )?;
    let path_start = FILE_EVENT_HEADER_SIZE;
    let secondary_path_start = path_start + FILE_PATH_ABI_MAX_BYTES;
    Ok(KernelFilePathEvent {
        kind: read_u32(raw, 0).expect("event length checked"),
        pid: read_u32(raw, 4).expect("event length checked"),
        tid: read_u32(raw, 8).expect("event length checked"),
        phase: read_u32(raw, 12).expect("event length checked"),
        result: read_i64(raw, 16).expect("event length checked"),
        trace_id: TraceId::new(read_u64(raw, 24).expect("event length checked")),
        observed_ktime_ns: read_u64(raw, 32).expect("event length checked"),
        fd: read_u32(raw, 40).expect("event length checked"),
        aux: read_u32(raw, 44).expect("event length checked"),
        path_size,
        path_flags: read_u32(raw, 52).expect("event length checked"),
        secondary_path_size,
        secondary_path_flags: read_u32(raw, 60).expect("event length checked"),
        path_max_bytes: read_u32(raw, 64).expect("event length checked"),
        arg0: read_u64(raw, 72).expect("event length checked"),
        arg1: read_u64(raw, 80).expect("event length checked"),
        arg2: read_u64(raw, 88).expect("event length checked"),
        arg3: read_u64(raw, 96).expect("event length checked"),
        arg4: read_u64(raw, 104).expect("event length checked"),
        arg5: read_u64(raw, 112).expect("event length checked"),
        pid_generation: read_u64(raw, 120).expect("event length checked"),
        path: if compact {
            Vec::new()
        } else {
            raw[path_start..path_start + path_size as usize].to_vec()
        },
        secondary_path: if compact || primary_path_only {
            Vec::new()
        } else {
            raw[secondary_path_start..secondary_path_start + secondary_path_size as usize].to_vec()
        },
    })
}

fn validate_path_size(
    label: &'static str,
    value: u32,
    max_bytes: usize,
) -> Result<(), LoaderError> {
    if value as usize > max_bytes {
        return Err(LoaderError::new(
            "decode_file_path",
            format!("{label} size {value} exceeds ABI maximum {max_bytes}"),
        ));
    }
    Ok(())
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

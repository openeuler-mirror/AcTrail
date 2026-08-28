//! File-path event wire decoding.

use model_core::ids::TraceId;

use super::{KernelFilePathEvent, read_i64, read_u32, read_u64};
use crate::loader::LoaderError;

pub(super) fn decode_file_path_event(raw: &[u8]) -> Result<KernelFilePathEvent, LoaderError> {
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

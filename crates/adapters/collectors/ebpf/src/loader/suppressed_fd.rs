//! Suppressed-fd map ABI and low-kernel inheritance index maintenance.

use std::ffi::OsStr;

use config_core::daemon::EbpfCollectorConfig;
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;
use model_core::process::{KernelProcessCoordinates, ProcessSuppressedFd, SuppressedFdPurpose};

use super::LoaderError;

const SUPPRESSED_FD_INDEX_SLOT_MAX: u32 = 64;
const SUPPRESSED_FD_KEY_SIZE: usize = std::mem::size_of::<SuppressedFdKeyLayout>();
const SUPPRESSED_FD_VALUE_SIZE: usize = std::mem::size_of::<SuppressedFdValueLayout>();
const SUPPRESSED_FD_INDEX_KEY_SIZE: usize = std::mem::size_of::<SuppressedFdIndexKeyLayout>();
const SUPPRESSED_FD_INDEX_VALUE_SIZE: usize = std::mem::size_of::<SuppressedFdIndexValueLayout>();
const SUPPRESSED_FD_CONFIG_KEY: u32 = 0;
const SUPPRESSED_FD_CONFIG_VALUE_SIZE: usize = std::mem::size_of::<SuppressedFdConfigLayout>();
const SUPPRESSED_FD_PURPOSE_TLS_SYNC_EVENT: u32 = 1;
const SUPPRESSED_FD_PURPOSE_INTERNAL_UPLOAD: u32 = 2;
const SUPPRESSED_FD_PURPOSE_INTERNAL_CONTROL: u32 = 3;

#[repr(C)]
struct SuppressedFdConfigLayout {
    index_slots_per_process: u32,
}

#[repr(C)]
struct SuppressedFdKeyLayout {
    pid: u32,
    fd: u32,
    generation: u64,
}

#[repr(C)]
struct SuppressedFdValueLayout {
    trace_id: u64,
    purpose: u32,
}

#[repr(C)]
struct SuppressedFdIndexKeyLayout {
    pid: u32,
    slot: u32,
    generation: u64,
}

#[repr(C)]
struct SuppressedFdIndexValueLayout {
    trace_id: u64,
    fd: u32,
    purpose: u32,
}

pub(crate) fn validate_config(config: &EbpfCollectorConfig) -> Result<(), LoaderError> {
    let slots = config.suppressed_fd_index_slots_per_process;
    if slots == 0 || slots > SUPPRESSED_FD_INDEX_SLOT_MAX {
        return Err(LoaderError::new(
            "suppressed_fd_config",
            format!(
                "suppressed_fd_index_slots_per_process must be in 1..={SUPPRESSED_FD_INDEX_SLOT_MAX}, got {slots}"
            ),
        ));
    }
    Ok(())
}

pub(crate) fn configure_config_map(
    object: &Object,
    config: &EbpfCollectorConfig,
) -> Result<(), LoaderError> {
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("suppressed_fd_config"))
        .ok_or_else(|| {
            LoaderError::new(
                "suppressed_fd_config",
                "suppressed_fd_config map is missing",
            )
        })
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("suppressed_fd_config", error.to_string()))
        })?;
    let key = SUPPRESSED_FD_CONFIG_KEY.to_ne_bytes();
    let mut value = [0_u8; SUPPRESSED_FD_CONFIG_VALUE_SIZE];
    value[0..4].copy_from_slice(&config.suppressed_fd_index_slots_per_process.to_ne_bytes());
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("suppressed_fd_config", error.to_string()))
}

pub(crate) fn suppress_fd(
    suppressed_fds: &MapHandle,
    suppressed_fd_index: &MapHandle,
    index_slots_per_process: u32,
    trace_id: TraceId,
    suppressed_fd: &ProcessSuppressedFd,
) -> Result<(), LoaderError> {
    let fd = suppressed_fd_u32(suppressed_fd.fd)?;
    let key = suppressed_fd_key(&suppressed_fd.process, fd)?;
    let value = suppressed_fd_value(trace_id, suppressed_fd.purpose);
    suppressed_fds
        .update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("suppress_fd", error.to_string()))?;
    if let Err(error) = upsert_index(
        suppressed_fd_index,
        index_slots_per_process,
        &suppressed_fd.process,
        fd,
        &value,
    ) {
        let _ = suppressed_fds.delete(&key);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn unsuppress_fd(
    suppressed_fds: &MapHandle,
    suppressed_fd_index: &MapHandle,
    index_slots_per_process: u32,
    process: &KernelProcessCoordinates,
    fd: i32,
) -> Result<(), LoaderError> {
    let fd = suppressed_fd_u32(fd)?;
    let key = suppressed_fd_key(process, fd)?;
    delete_index(
        suppressed_fd_index,
        index_slots_per_process,
        process.pid,
        process.start_time,
        fd,
    )?;
    if suppressed_fds
        .lookup(&key, MapFlags::ANY)
        .map_err(|error| LoaderError::new("lookup_suppressed_fd", error.to_string()))?
        .is_none()
    {
        return Ok(());
    }
    suppressed_fds
        .delete(&key)
        .map_err(|error| LoaderError::new("unsuppress_fd", error.to_string()))
}

pub(crate) fn sweep_process(
    suppressed_fds: &MapHandle,
    suppressed_fd_index: &MapHandle,
    pid: u32,
    generation: u64,
) -> Result<(), LoaderError> {
    for key in suppressed_fds.keys().collect::<Vec<_>>() {
        let Some(parsed) = parse_suppressed_fd_key(&key) else {
            continue;
        };
        if parsed.pid == pid && parsed.generation == generation {
            suppressed_fds
                .delete(&key)
                .map_err(|error| LoaderError::new("sweep_suppressed_fds", error.to_string()))?;
        }
    }
    for key in suppressed_fd_index.keys().collect::<Vec<_>>() {
        let Some(parsed) = parse_suppressed_fd_index_key(&key) else {
            continue;
        };
        if parsed.pid == pid && parsed.generation == generation {
            suppressed_fd_index.delete(&key).map_err(|error| {
                LoaderError::new("sweep_suppressed_fd_index", error.to_string())
            })?;
        }
    }
    Ok(())
}

pub(crate) fn sweep_trace(
    suppressed_fds: &MapHandle,
    suppressed_fd_index: &MapHandle,
    trace_id: TraceId,
) -> Result<(), LoaderError> {
    for key in suppressed_fds.keys().collect::<Vec<_>>() {
        let Some(value) = suppressed_fds
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("lookup_suppressed_fd", error.to_string()))?
        else {
            continue;
        };
        if parse_trace_id(&value) == Some(trace_id.get()) {
            suppressed_fds
                .delete(&key)
                .map_err(|error| LoaderError::new("sweep_suppressed_fds", error.to_string()))?;
        }
    }
    for key in suppressed_fd_index.keys().collect::<Vec<_>>() {
        let Some(value) = suppressed_fd_index
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("lookup_suppressed_fd_index", error.to_string()))?
        else {
            continue;
        };
        if parse_trace_id(&value) == Some(trace_id.get()) {
            suppressed_fd_index.delete(&key).map_err(|error| {
                LoaderError::new("sweep_suppressed_fd_index", error.to_string())
            })?;
        }
    }
    Ok(())
}

fn upsert_index(
    suppressed_fd_index: &MapHandle,
    index_slots_per_process: u32,
    process: &KernelProcessCoordinates,
    fd: u32,
    value: &[u8; SUPPRESSED_FD_VALUE_SIZE],
) -> Result<(), LoaderError> {
    if process.start_time == 0 {
        return Err(LoaderError::new(
            "suppress_fd",
            "suppressed fd index requires a non-zero process generation",
        ));
    }
    let index_value = suppressed_fd_index_value(fd, value)?;
    for slot in 0..index_slots_per_process {
        let key = suppressed_fd_index_key(process.pid, process.start_time, slot);
        match suppressed_fd_index
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("lookup_suppressed_fd_index", error.to_string()))?
        {
            Some(existing) if parse_index_fd(&existing) == Some(fd) => {
                return suppressed_fd_index
                    .update(&key, &index_value, MapFlags::ANY)
                    .map_err(|error| LoaderError::new("suppress_fd_index", error.to_string()));
            }
            Some(_) => {}
            None => {
                return suppressed_fd_index
                    .update(&key, &index_value, MapFlags::ANY)
                    .map_err(|error| LoaderError::new("suppress_fd_index", error.to_string()));
            }
        }
    }
    Err(LoaderError::new(
        "suppress_fd_index",
        format!(
            "suppressed fd index slots exhausted for pid {} generation {}: configured slots {}",
            process.pid, process.start_time, index_slots_per_process
        ),
    ))
}

fn delete_index(
    suppressed_fd_index: &MapHandle,
    index_slots_per_process: u32,
    pid: u32,
    generation: u64,
    fd: u32,
) -> Result<(), LoaderError> {
    for slot in 0..index_slots_per_process {
        let key = suppressed_fd_index_key(pid, generation, slot);
        let Some(existing) = suppressed_fd_index
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("lookup_suppressed_fd_index", error.to_string()))?
        else {
            continue;
        };
        if parse_index_fd(&existing) == Some(fd) {
            suppressed_fd_index
                .delete(&key)
                .map_err(|error| LoaderError::new("unsuppress_fd_index", error.to_string()))?;
            return Ok(());
        }
    }
    Ok(())
}

struct SuppressedFdKeyParts {
    pid: u32,
    generation: u64,
}

fn suppressed_fd_u32(fd: i32) -> Result<u32, LoaderError> {
    u32::try_from(fd).map_err(|_| {
        LoaderError::new(
            "suppress_fd",
            format!("suppressed fd must be non-negative: {fd}"),
        )
    })
}

fn suppressed_fd_key(
    process: &KernelProcessCoordinates,
    fd: u32,
) -> Result<[u8; SUPPRESSED_FD_KEY_SIZE], LoaderError> {
    if process.start_time == 0 {
        return Err(LoaderError::new(
            "suppress_fd",
            "suppressed fd requires a non-zero process generation",
        ));
    }
    let mut key = [0_u8; SUPPRESSED_FD_KEY_SIZE];
    key[0..4].copy_from_slice(&process.pid.to_ne_bytes());
    key[4..8].copy_from_slice(&fd.to_ne_bytes());
    key[8..16].copy_from_slice(&process.start_time.to_ne_bytes());
    Ok(key)
}

fn suppressed_fd_index_key(
    pid: u32,
    generation: u64,
    slot: u32,
) -> [u8; SUPPRESSED_FD_INDEX_KEY_SIZE] {
    let mut key = [0_u8; SUPPRESSED_FD_INDEX_KEY_SIZE];
    key[0..4].copy_from_slice(&pid.to_ne_bytes());
    key[4..8].copy_from_slice(&slot.to_ne_bytes());
    key[8..16].copy_from_slice(&generation.to_ne_bytes());
    key
}

fn suppressed_fd_value(
    trace_id: TraceId,
    purpose: SuppressedFdPurpose,
) -> [u8; SUPPRESSED_FD_VALUE_SIZE] {
    let mut value = [0_u8; SUPPRESSED_FD_VALUE_SIZE];
    value[0..8].copy_from_slice(&trace_id.get().to_ne_bytes());
    value[8..12].copy_from_slice(&suppressed_fd_purpose_code(purpose).to_ne_bytes());
    value
}

fn suppressed_fd_index_value(
    fd: u32,
    value: &[u8; SUPPRESSED_FD_VALUE_SIZE],
) -> Result<[u8; SUPPRESSED_FD_INDEX_VALUE_SIZE], LoaderError> {
    let trace_id = value
        .get(0..8)
        .ok_or_else(|| LoaderError::new("suppress_fd_index", "missing trace id"))?;
    let purpose = value
        .get(8..12)
        .ok_or_else(|| LoaderError::new("suppress_fd_index", "missing purpose"))?;
    let mut index_value = [0_u8; SUPPRESSED_FD_INDEX_VALUE_SIZE];
    index_value[0..8].copy_from_slice(trace_id);
    index_value[8..12].copy_from_slice(&fd.to_ne_bytes());
    index_value[12..16].copy_from_slice(purpose);
    Ok(index_value)
}

fn parse_suppressed_fd_key(key: &[u8]) -> Option<SuppressedFdKeyParts> {
    if key.len() != SUPPRESSED_FD_KEY_SIZE {
        return None;
    }
    Some(SuppressedFdKeyParts {
        pid: u32::from_ne_bytes(key[0..4].try_into().ok()?),
        generation: u64::from_ne_bytes(key[8..16].try_into().ok()?),
    })
}

fn parse_suppressed_fd_index_key(key: &[u8]) -> Option<SuppressedFdKeyParts> {
    if key.len() != SUPPRESSED_FD_INDEX_KEY_SIZE {
        return None;
    }
    Some(SuppressedFdKeyParts {
        pid: u32::from_ne_bytes(key[0..4].try_into().ok()?),
        generation: u64::from_ne_bytes(key[8..16].try_into().ok()?),
    })
}

fn parse_trace_id(value: &[u8]) -> Option<u64> {
    Some(u64::from_ne_bytes(value.get(0..8)?.try_into().ok()?))
}

fn parse_index_fd(value: &[u8]) -> Option<u32> {
    if value.len() != SUPPRESSED_FD_INDEX_VALUE_SIZE {
        return None;
    }
    Some(u32::from_ne_bytes(value[8..12].try_into().ok()?))
}

fn suppressed_fd_purpose_code(purpose: SuppressedFdPurpose) -> u32 {
    match purpose {
        SuppressedFdPurpose::TlsSyncEvent => SUPPRESSED_FD_PURPOSE_TLS_SYNC_EVENT,
        SuppressedFdPurpose::InternalUpload => SUPPRESSED_FD_PURPOSE_INTERNAL_UPLOAD,
        SuppressedFdPurpose::InternalControl => SUPPRESSED_FD_PURPOSE_INTERNAL_CONTROL,
    }
}

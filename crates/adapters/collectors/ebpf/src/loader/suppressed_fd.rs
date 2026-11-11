//! Suppressed-fd map ABI and low-kernel inheritance index maintenance.

use config_core::daemon::EbpfCollectorConfig;
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;
use std::os::unix::ffi::OsStrExt;

use super::LoaderError;
use super::object::map_handle;

pub(super) const SUPPRESSED_FD_INDEX_SLOT_MAX: u32 = 64;
const SUPPRESSED_FD_KEY_SIZE: usize = std::mem::size_of::<SuppressedFdKeyLayout>();
const SUPPRESSED_FD_INDEX_KEY_SIZE: usize = std::mem::size_of::<SuppressedFdIndexKeyLayout>();
const SUPPRESSED_FD_CONFIG_KEY: u32 = 0;

pub(super) fn configure_maps(
    object: &Object,
    collector: &EbpfCollectorConfig,
    config: &config_core::daemon::PayloadTlsConfig,
) -> Result<(), LoaderError> {
    let enabled = config.enabled
        && config.capture_backend == config_core::daemon::PayloadTlsCaptureBackend::TlsSync;
    let slots = if enabled {
        collector.suppressed_fd_index_slots_per_process
    } else {
        0
    };
    map_handle(object, "suppressed_fd_config", "suppressed_fd_config")?
        .update(
            &SUPPRESSED_FD_CONFIG_KEY.to_ne_bytes(),
            &slots.to_ne_bytes(),
            MapFlags::ANY,
        )
        .map_err(|error| LoaderError::new("suppressed_fd_config", error.to_string()))?;
    let mut value = [0_u8; 4 + 108];
    if enabled {
        let path = config.sync_event_socket_path.as_os_str().as_bytes();
        if path.is_empty() || path.len() >= 108 || path.contains(&0) {
            return Err(LoaderError::new(
                "tls_sync_socket_path",
                "TLS-sync socket path must contain 1..=107 non-NUL bytes",
            ));
        }
        value[..4].copy_from_slice(&(path.len() as u32).to_ne_bytes());
        value[4..4 + path.len()].copy_from_slice(path);
    }
    map_handle(object, "internal_socket_config", "internal_socket_config")?
        .update(&0_u32.to_ne_bytes(), &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("internal_socket_config", error.to_string()))
}

#[repr(C)]
struct SuppressedFdKeyLayout {
    pid: u32,
    fd: u32,
    generation: u64,
}

#[repr(C)]
struct SuppressedFdIndexKeyLayout {
    pid: u32,
    slot: u32,
    generation: u64,
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

struct SuppressedFdKeyParts {
    pid: u32,
    generation: u64,
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

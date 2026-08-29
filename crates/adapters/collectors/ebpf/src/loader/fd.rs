//! Unified fd-table category configuration.

use std::ffi::OsStr;

use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};
use model_core::capability::Capability;

use crate::loader::{AttachPlan, LoaderError};

// Should be same with enum actrail_fd_category_flag in bpf/fd/types.h.
const FD_CATEGORY_FLAG_NET: u32 = 1 << 1;
const FD_CATEGORY_FLAG_IPC_UNIX_SOCKET: u32 = 1 << 2;
const FD_CATEGORY_FLAG_IPC_PIPE: u32 = 1 << 3;
const FD_CATEGORY_FLAG_FILE: u32 = 1 << 4;
const FD_INDEX_HARD_MAX_ENTRIES: u32 = 64;

pub fn validate_fd_config(
    config: &config_core::daemon::EbpfCollectorConfig,
) -> Result<(), LoaderError> {
    let entries = config.fd_per_process_max_entries;
    if entries == 0 || entries > FD_INDEX_HARD_MAX_ENTRIES {
        return Err(LoaderError::new(
            "fd_per_process_max_entries",
            format!(
                "fd_per_process_max_entries must be in 1..={FD_INDEX_HARD_MAX_ENTRIES}, got {entries}"
            ),
        ));
    }
    Ok(())
}

fn fd_category_config_value(attach_plan: &AttachPlan) -> u32 {
    let mut flags = 0u32;
    if attach_plan.contains(&Capability::NetTransport) {
        flags |= FD_CATEGORY_FLAG_NET;
    }
    if attach_plan.contains(&Capability::IpcUnixSocket) {
        flags |= FD_CATEGORY_FLAG_IPC_UNIX_SOCKET;
    }
    if attach_plan.contains(&Capability::IpcPipeFifo) {
        flags |= FD_CATEGORY_FLAG_IPC_PIPE;
    }
    if attach_plan.contains(&Capability::FsAccessBasic) {
        flags |= FD_CATEGORY_FLAG_FILE;
    }
    flags
}

/// Write the enabled-category bitmask into `fd_category_config` so entry
/// probes skip fds whose category is not collected by the attach plan.
pub fn configure_fd_category_config_map(
    object: &Object,
    attach_plan: &AttachPlan,
    config: &config_core::daemon::EbpfCollectorConfig,
) -> Result<(), LoaderError> {
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("fd_category_config"))
        .ok_or_else(|| LoaderError::new("fd_category_config", "fd_category_config map is missing"))
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("fd_category_config", error.to_string()))
        })?;
    let key = 0_u32.to_ne_bytes();
    let mut value = [0_u8; 8];
    value[0..4].copy_from_slice(&fd_category_config_value(attach_plan).to_ne_bytes());
    value[4..8].copy_from_slice(&config.fd_per_process_max_entries.to_ne_bytes());
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("fd_category_config", error.to_string()))
}

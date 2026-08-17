//! File-path capture configuration for eBPF events.

use std::ffi::OsStr;

use config_core::daemon::EbpfCollectorConfig;
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};
use model_core::capability::Capability;

use crate::loader::{AttachPlan, LoaderError};

pub const FILE_PATH_MAX_BYTES: u32 = 255;

pub fn validate_file_config(config: &EbpfCollectorConfig) -> Result<(), LoaderError> {
    if config.file_path_max_bytes > FILE_PATH_MAX_BYTES {
        return Err(LoaderError::new(
            "file_path_config",
            format!(
                "file_path_max_bytes {} exceeds compiled ABI maximum {}",
                config.file_path_max_bytes, FILE_PATH_MAX_BYTES
            ),
        ));
    }
    Ok(())
}

pub fn configure_file_config_map(
    object: &Object,
    config: &EbpfCollectorConfig,
    attach_plan: &AttachPlan,
) -> Result<(), LoaderError> {
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("file_config"))
        .ok_or_else(|| LoaderError::new("file_path_config", "file_config map is missing"))
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("file_path_config", error.to_string()))
        })?;
    let key = 0_u32.to_ne_bytes();
    let mut value = [0_u8; 8];
    value[0..4].copy_from_slice(&config.file_path_max_bytes.to_ne_bytes());
    let fs_capture_enabled =
        config.file_path_capture_enabled && attach_plan.contains(&Capability::FsAccessBasic);
    let ipc_lineage_enabled = attach_plan.contains(&Capability::IpcPipeFifo)
        || attach_plan.contains(&Capability::IpcUnixSocket);
    let capture_enabled = fs_capture_enabled || ipc_lineage_enabled;
    value[4..8].copy_from_slice(&u32::from(capture_enabled).to_ne_bytes());
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("file_path_config", error.to_string()))
}

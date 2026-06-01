//! File-path capture configuration for eBPF events.

use std::ffi::OsStr;

use config_core::daemon::EbpfCollectorConfig;
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};

use crate::loader::LoaderError;

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
    let value = config.file_path_max_bytes.to_ne_bytes();
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| LoaderError::new("file_path_config", error.to_string()))
}

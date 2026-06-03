//! Stdio payload map configuration.

use std::ffi::OsStr;

use config_core::daemon::PayloadStdioConfig;
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};

use crate::loader::LoaderError;

const STDIO_PAYLOAD_MAX_SEGMENT_BYTES: u32 = 4095;
const STDIO_PAYLOAD_CONFIG_KEY: u32 = 0;
const STDIO_PAYLOAD_CONFIG_FIELDS: usize = 5;
const STDIO_PAYLOAD_CONFIG_FIELD_SIZE: usize = std::mem::size_of::<u32>();
const STDIO_PAYLOAD_CONFIG_VALUE_SIZE: usize =
    STDIO_PAYLOAD_CONFIG_FIELDS * STDIO_PAYLOAD_CONFIG_FIELD_SIZE;

pub fn validate_payload_config(config: &PayloadStdioConfig) -> Result<(), LoaderError> {
    if !config.enabled {
        return Ok(());
    }
    if !config.capture_stdin && !config.capture_stdout && !config.capture_stderr {
        return Err(LoaderError::new(
            "payload_stdio_config",
            "payload_stdio_enabled=true requires at least one enabled stdio stream",
        ));
    }
    if config.max_segment_bytes > STDIO_PAYLOAD_MAX_SEGMENT_BYTES {
        return Err(LoaderError::new(
            "payload_stdio_config",
            format!(
                "payload_stdio_max_segment_bytes {} exceeds compiled ABI maximum {}",
                config.max_segment_bytes, STDIO_PAYLOAD_MAX_SEGMENT_BYTES
            ),
        ));
    }
    Ok(())
}

pub fn configure_payload_stdio_map(
    object: &Object,
    config: &PayloadStdioConfig,
) -> Result<(), LoaderError> {
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("payload_stdio_config"))
        .ok_or_else(|| {
            LoaderError::new(
                "payload_stdio_config",
                "payload_stdio_config map is missing",
            )
        })
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("payload_stdio_config", error.to_string()))
        })?;
    let value = payload_stdio_config_value(config);
    map.update(
        &STDIO_PAYLOAD_CONFIG_KEY.to_ne_bytes(),
        &value,
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("payload_stdio_config", error.to_string()))
}

fn payload_stdio_config_value(
    config: &PayloadStdioConfig,
) -> [u8; STDIO_PAYLOAD_CONFIG_VALUE_SIZE] {
    let fields = [
        bool_field(config.enabled),
        bool_field(config.capture_stdin),
        bool_field(config.capture_stdout),
        bool_field(config.capture_stderr),
        config.max_segment_bytes,
    ];
    let mut value = [0_u8; STDIO_PAYLOAD_CONFIG_VALUE_SIZE];
    for (index, field) in fields.into_iter().enumerate() {
        let start = index * STDIO_PAYLOAD_CONFIG_FIELD_SIZE;
        let end = start + STDIO_PAYLOAD_CONFIG_FIELD_SIZE;
        value[start..end].copy_from_slice(&field.to_ne_bytes());
    }
    value
}

fn bool_field(value: bool) -> u32 {
    if value { 1 } else { 0 }
}

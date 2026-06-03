//! Socket plaintext payload map configuration.

use std::ffi::OsStr;

use config_core::daemon::{PayloadSocketCaptureBackend, PayloadSocketConfig};
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};

use crate::loader::LoaderError;

const SOCKET_PAYLOAD_MAX_SEGMENT_BYTES: u32 = 4_095;
const SOCKET_PAYLOAD_CONFIG_KEY: u32 = 0;
const SOCKET_PAYLOAD_CONFIG_FIELDS: usize = 3;
const SOCKET_PAYLOAD_CONFIG_FIELD_SIZE: usize = std::mem::size_of::<u32>();
const SOCKET_PAYLOAD_CONFIG_VALUE_SIZE: usize =
    SOCKET_PAYLOAD_CONFIG_FIELDS * SOCKET_PAYLOAD_CONFIG_FIELD_SIZE;

pub fn validate_payload_config(config: &PayloadSocketConfig) -> Result<(), LoaderError> {
    if !config.enabled {
        return Ok(());
    }
    if config.max_segment_bytes > SOCKET_PAYLOAD_MAX_SEGMENT_BYTES {
        return Err(LoaderError::new(
            "payload_socket_config",
            format!(
                "payload_socket_max_segment_bytes {} exceeds compiled ABI maximum {}",
                config.max_segment_bytes, SOCKET_PAYLOAD_MAX_SEGMENT_BYTES
            ),
        ));
    }
    Ok(())
}

pub fn configure_payload_socket_map(
    object: &Object,
    config: &PayloadSocketConfig,
) -> Result<(), LoaderError> {
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("payload_socket_config"))
        .ok_or_else(|| {
            LoaderError::new(
                "payload_socket_config",
                "payload_socket_config map is missing",
            )
        })
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("payload_socket_config", error.to_string()))
        })?;
    let value = payload_socket_config_value(config);
    map.update(
        &SOCKET_PAYLOAD_CONFIG_KEY.to_ne_bytes(),
        &value,
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("payload_socket_config", error.to_string()))
}

fn payload_socket_config_value(
    config: &PayloadSocketConfig,
) -> [u8; SOCKET_PAYLOAD_CONFIG_VALUE_SIZE] {
    let fields = [
        bool_field(config.enabled),
        config.max_segment_bytes,
        bool_field(config.capture_backend == PayloadSocketCaptureBackend::BpfCopySeccompFallback),
    ];
    let mut value = [0_u8; SOCKET_PAYLOAD_CONFIG_VALUE_SIZE];
    for (index, field) in fields.into_iter().enumerate() {
        let start = index * SOCKET_PAYLOAD_CONFIG_FIELD_SIZE;
        let end = start + SOCKET_PAYLOAD_CONFIG_FIELD_SIZE;
        value[start..end].copy_from_slice(&field.to_ne_bytes());
    }
    value
}

fn bool_field(value: bool) -> u32 {
    if value { 1 } else { 0 }
}

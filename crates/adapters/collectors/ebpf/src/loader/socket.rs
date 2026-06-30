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
const SOCKET_PAYLOAD_FD_KEY_FIELDS: usize = 2;
const SOCKET_PAYLOAD_FD_KEY_SIZE: usize = SOCKET_PAYLOAD_FD_KEY_FIELDS * std::mem::size_of::<u32>();

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

pub(crate) fn lookup_fd_generation(
    map: &MapHandle,
    pid: u32,
    fd: u32,
) -> Result<Option<u32>, LoaderError> {
    let key = socket_fd_key(pid, fd);
    map.lookup(&key, MapFlags::ANY)
        .map_err(|error| LoaderError::new("lookup_socket_fd_generation", error.to_string()))?
        .map(|value| read_u32_value(&value))
        .transpose()
}

fn socket_fd_key(pid: u32, fd: u32) -> [u8; SOCKET_PAYLOAD_FD_KEY_SIZE] {
    let mut key = [0_u8; SOCKET_PAYLOAD_FD_KEY_SIZE];
    key[..std::mem::size_of::<u32>()].copy_from_slice(&pid.to_ne_bytes());
    key[std::mem::size_of::<u32>()..].copy_from_slice(&fd.to_ne_bytes());
    key
}

fn read_u32_value(value: &[u8]) -> Result<u32, LoaderError> {
    value
        .get(..std::mem::size_of::<u32>())
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_ne_bytes)
        .ok_or_else(|| {
            LoaderError::new(
                "lookup_socket_fd_generation",
                format!("unexpected socket fd generation value size {}", value.len()),
            )
        })
}

//! Shared libbpf object helpers for runtime loading.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::rc::Rc;

use config_core::daemon::{EbpfCollectorConfig, PayloadConfig};
use libbpf_rs::{MapCore, MapHandle, Object, RingBuffer, RingBufferBuilder};

use super::LoaderError;

pub(crate) fn ring_buffer_max_bytes(config: &EbpfCollectorConfig, payload: &PayloadConfig) -> u32 {
    let mut max_bytes = config.event_ring_buffer_max_bytes;
    if payload.tls.enabled {
        max_bytes = max_bytes.max(payload.tls.ring_buffer_bytes);
    }
    if payload.stdio.enabled {
        max_bytes = max_bytes.max(payload.stdio.ring_buffer_bytes);
    }
    if payload.socket.enabled {
        max_bytes = max_bytes.max(payload.socket.ring_buffer_bytes);
    }
    max_bytes
}

pub(crate) fn build_ring_buffer(
    events_map: &MapHandle,
    events: Rc<RefCell<Vec<Vec<u8>>>>,
) -> Result<RingBuffer<'static>, LoaderError> {
    let mut builder = RingBufferBuilder::new();
    builder
        .add(events_map, move |raw| {
            events.borrow_mut().push(raw.to_vec());
            0
        })
        .map_err(|error| LoaderError::new("ring_buffer", error.to_string()))?;
    builder
        .build()
        .map_err(|error| LoaderError::new("ring_buffer", error.to_string()))
}

pub(crate) fn map_handle(
    object: &Object,
    map_name: &'static str,
    stage: &'static str,
) -> Result<MapHandle, LoaderError> {
    object
        .maps()
        .find(|map| map.name() == OsStr::new(map_name))
        .ok_or_else(|| LoaderError::new(stage, format!("{map_name} map is missing")))
        .and_then(|map| {
            MapHandle::try_from(&map).map_err(|error| LoaderError::new(stage, error.to_string()))
        })
}

pub(crate) fn resize_map(
    open_object: &mut libbpf_rs::OpenObject,
    map_name: &str,
    max_entries: u32,
) -> Result<(), LoaderError> {
    let mut map = open_object
        .maps_mut()
        .find(|map| map.name() == OsStr::new(map_name))
        .ok_or_else(|| LoaderError::new("resize_map", format!("map {map_name} is missing")))?;
    map.set_max_entries(max_entries)
        .map_err(|error| LoaderError::new("resize_map", error.to_string()))
}

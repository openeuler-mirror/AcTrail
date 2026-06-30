//! Shared libbpf object helpers for runtime loading.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::rc::Rc;
#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
use std::sync::Arc;
#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
use std::sync::atomic::{AtomicU64, Ordering};

use config_core::daemon::{EbpfCollectorConfig, PayloadConfig};
use libbpf_rs::{MapCore, MapHandle, Object};
#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
use libbpf_rs::{PerfBuffer, PerfBufferBuilder};
#[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
use libbpf_rs::{RingBuffer, RingBufferBuilder};

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

#[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
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

pub(crate) enum EventBuffer {
    #[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
    Ring(RingBuffer<'static>),
    #[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
    Perf {
        buffer: PerfBuffer<'static>,
        lost: Arc<AtomicU64>,
    },
}

impl EventBuffer {
    pub(crate) fn build(
        events_map: &MapHandle,
        events: Rc<RefCell<Vec<Vec<u8>>>>,
        buffer_bytes: u32,
    ) -> Result<Self, LoaderError> {
        #[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
        {
            let (buffer, lost) = build_perf_buffer(events_map, events, buffer_bytes)?;
            return Ok(Self::Perf { buffer, lost });
        }
        #[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
        {
            let _ = buffer_bytes;
            build_ring_buffer(events_map, events).map(Self::Ring)
        }
    }

    pub(crate) fn consume(&self) -> Result<(), LoaderError> {
        match self {
            #[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
            Self::Ring(buffer) => buffer
                .consume()
                .map_err(|error| LoaderError::new("consume_ring_buffer", error.to_string())),
            #[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
            Self::Perf { buffer, .. } => buffer
                .consume()
                .map_err(|error| LoaderError::new("consume_perf_buffer", error.to_string())),
        }
    }

    pub(crate) fn epoll_fd(&self) -> i32 {
        match self {
            #[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
            Self::Ring(buffer) => buffer.epoll_fd(),
            #[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
            Self::Perf { buffer, .. } => buffer.epoll_fd(),
        }
    }

    pub(crate) fn lost_count(&self) -> u64 {
        match self {
            #[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
            Self::Ring(_) => 0,
            #[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
            Self::Perf { lost, .. } => lost.load(Ordering::Relaxed),
        }
    }
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn build_perf_buffer(
    events_map: &MapHandle,
    events: Rc<RefCell<Vec<Vec<u8>>>>,
    buffer_bytes: u32,
) -> Result<(PerfBuffer<'static>, Arc<AtomicU64>), LoaderError> {
    let callback_events = Rc::clone(&events);
    let lost = Arc::new(AtomicU64::new(0));
    let callback_lost = Arc::clone(&lost);
    let pages = perf_pages_for_bytes(buffer_bytes)?;
    let buffer = PerfBufferBuilder::new(events_map)
        .sample_cb(move |_cpu, raw| {
            callback_events
                .borrow_mut()
                .push(perf_sample_payload(raw).to_vec());
        })
        .lost_cb(move |_cpu, count| {
            callback_lost.fetch_add(count, Ordering::Relaxed);
        })
        .pages(pages)
        .build()
        .map_err(|error| LoaderError::new("perf_buffer", error.to_string()))?;
    Ok((buffer, lost))
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn perf_sample_payload(raw: &[u8]) -> &[u8] {
    strip_perf_raw_size_prefix(raw)
        .or_else(|| strip_perf_trailing_padding(raw))
        .unwrap_or(raw)
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn strip_perf_raw_size_prefix(raw: &[u8]) -> Option<&[u8]> {
    let declared_size = read_u32(raw, 0)? as usize;
    let payload = raw.get(4..)?;
    if declared_size != payload.len() {
        return None;
    }
    let kind = read_u32(raw, 4)?;
    if known_event_kind(kind) {
        Some(payload)
    } else {
        None
    }
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn strip_perf_trailing_padding(raw: &[u8]) -> Option<&[u8]> {
    let payload = raw.get(..raw.len().checked_sub(4)?)?;
    let kind = read_u32(payload, 0)?;
    if known_event_size(kind, payload.len()) {
        Some(payload)
    } else {
        None
    }
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn known_event_size(kind: u32, size: usize) -> bool {
    const OBSERVATION_EVENT_SIZE: usize = 112;
    const EXEC_EVENT_SIZE: usize = 632;
    const TLS_FIXED_EVENT_SIZE: usize = 80;
    const TLS_DIRECT_CAPTURE_EVENT_SIZE: usize = 80 + 4_194_304;
    const FILE_EVENT_HEADER_SIZE: usize = 128;
    const FILE_EVENT_PRIMARY_PATH_SIZE: usize = FILE_EVENT_HEADER_SIZE + 256;
    const FILE_EVENT_SIZE: usize = FILE_EVENT_HEADER_SIZE + 256 * 2;
    const STDIO_EVENT_SIZE: usize = 72 + 4_096;
    const SOCKET_EVENT_SIZE: usize = 72 + 4_096;
    const SOCKET_COMPLETION_EVENT_SIZE: usize = 88;

    if !known_event_kind(kind) {
        return false;
    }

    match kind {
        1 | 3 | 4 | 100..=105 => size == OBSERVATION_EVENT_SIZE,
        2 => size == EXEC_EVENT_SIZE,
        201 | 202 | 204 => size == TLS_FIXED_EVENT_SIZE,
        203 => size == TLS_DIRECT_CAPTURE_EVENT_SIZE,
        300..=307 => {
            matches!(
                size,
                FILE_EVENT_HEADER_SIZE | FILE_EVENT_PRIMARY_PATH_SIZE | FILE_EVENT_SIZE
            )
        }
        400 => size == STDIO_EVENT_SIZE,
        500 => size == SOCKET_EVENT_SIZE,
        501 => size == SOCKET_COMPLETION_EVENT_SIZE,
        _ => false,
    }
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn known_event_kind(kind: u32) -> bool {
    matches!(
        kind,
        1..=4 | 100..=105 | 201..=204 | 300..=307 | 400 | 500 | 501
    )
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn read_u32(raw: &[u8], offset: usize) -> Option<u32> {
    raw.get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_ne_bytes)
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn perf_pages_for_bytes(buffer_bytes: u32) -> Result<usize, LoaderError> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(LoaderError::new(
            "perf_buffer",
            format!("invalid system page size {page_size}"),
        ));
    }
    let page_size = usize::try_from(page_size)
        .map_err(|error| LoaderError::new("perf_buffer", format!("page size overflow: {error}")))?;
    let bytes = usize::try_from(buffer_bytes).map_err(|error| {
        LoaderError::new("perf_buffer", format!("buffer size overflow: {error}"))
    })?;
    let pages = bytes.div_ceil(page_size).max(1).next_power_of_two();
    Ok(pages)
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
pub(crate) fn event_map_max_entries(_buffer_bytes: u32) -> Result<u32, LoaderError> {
    let cpus = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_CONF) };
    if cpus <= 0 {
        return Err(LoaderError::new(
            "perf_buffer",
            format!("invalid configured CPU count {cpus}"),
        ));
    }
    u32::try_from(cpus)
        .map_err(|error| LoaderError::new("perf_buffer", format!("CPU count overflow: {error}")))
}

#[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
pub(crate) fn event_map_max_entries(buffer_bytes: u32) -> Result<u32, LoaderError> {
    Ok(buffer_bytes)
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

#[cfg(all(test, any(feature = "perf-buffer", actrail_event_transport_perf)))]
mod tests {
    use super::perf_sample_payload;

    #[test]
    fn perf_sample_payload_strips_raw_size_prefix() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&112u32.to_ne_bytes());
        raw.extend_from_slice(&1u32.to_ne_bytes());
        raw.resize(116, 0);

        let payload = perf_sample_payload(&raw);

        assert_eq!(payload.len(), 112);
        assert_eq!(&payload[..4], &1u32.to_ne_bytes());
    }

    #[test]
    fn perf_sample_payload_keeps_unprefixed_event() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&1u32.to_ne_bytes());
        raw.resize(112, 0);

        let payload = perf_sample_payload(&raw);

        assert_eq!(payload.len(), 112);
        assert_eq!(&payload[..4], &1u32.to_ne_bytes());
    }

    #[test]
    fn perf_sample_payload_strips_trailing_observation_padding() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&103u32.to_ne_bytes());
        raw.resize(116, 0);

        let payload = perf_sample_payload(&raw);

        assert_eq!(payload.len(), 112);
        assert_eq!(&payload[..4], &103u32.to_ne_bytes());
    }

    #[test]
    fn perf_sample_payload_strips_trailing_exec_padding() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&2u32.to_ne_bytes());
        raw.resize(636, 0);

        let payload = perf_sample_payload(&raw);

        assert_eq!(payload.len(), 632);
        assert_eq!(&payload[..4], &2u32.to_ne_bytes());
    }
}

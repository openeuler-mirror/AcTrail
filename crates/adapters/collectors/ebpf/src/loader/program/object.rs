//! Shared libbpf object helpers for runtime loading.

use std::ffi::OsStr;
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
#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
use super::abi::{
    FD_IO_EVENT_SIZE, LAUNCH_BINDING_FAILURE_EVENT_SIZE, NETWORK_EVENT_SIZE,
    PROCESS_EXEC_EVENT_SIZE, PROCESS_EXIT_EVENT_SIZE, PROCESS_FORK_EVENT_SIZE,
    PROCESS_SIGNAL_EVENT_SIZE, SOCKET_RELEASE_EVENT_SIZE,
};

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
    /// Build a transport whose sample callback forwards raw records to `sink`.
    ///
    /// The sink must not require `Rc` interior mutability, so the resulting
    /// buffer can be moved to a dedicated consumer thread: both transports are
    /// `Send` (libbpf's unsafe impls) and the sink is only ever invoked from
    /// the single thread that owns the buffer.
    pub(crate) fn build_with_sink<F>(
        events_map: &MapHandle,
        buffer_bytes: u32,
        sink: F,
    ) -> Result<Self, LoaderError>
    where
        F: FnMut(&[u8]) + 'static,
    {
        #[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
        {
            let (buffer, lost) = build_perf_buffer_with_sink(events_map, buffer_bytes, sink)?;
            return Ok(Self::Perf { buffer, lost });
        }
        #[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
        {
            let _ = buffer_bytes;
            build_ring_buffer_with_sink(events_map, sink).map(Self::Ring)
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

#[cfg(not(any(feature = "perf-buffer", actrail_event_transport_perf)))]
fn build_ring_buffer_with_sink<F>(
    events_map: &MapHandle,
    mut sink: F,
) -> Result<RingBuffer<'static>, LoaderError>
where
    F: FnMut(&[u8]) + 'static,
{
    let mut builder = RingBufferBuilder::new();
    builder
        .add(events_map, move |raw| {
            sink(raw);
            0
        })
        .map_err(|error| LoaderError::new("ring_buffer", error.to_string()))?;
    builder
        .build()
        .map_err(|error| LoaderError::new("ring_buffer", error.to_string()))
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn build_perf_buffer_with_sink<F>(
    events_map: &MapHandle,
    buffer_bytes: u32,
    mut sink: F,
) -> Result<(PerfBuffer<'static>, Arc<AtomicU64>), LoaderError>
where
    F: FnMut(&[u8]) + 'static,
{
    let lost = Arc::new(AtomicU64::new(0));
    let callback_lost = Arc::clone(&lost);
    let pages = perf_pages_for_bytes(buffer_bytes)?;
    let buffer = PerfBufferBuilder::new(events_map)
        .sample_cb(move |_cpu, raw| {
            sink(perf_sample_payload(raw));
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
    const TLS_PAYLOAD_FIXED_EVENT_SIZE: usize = 88;
    const TLS_DIAGNOSTIC_EVENT_SIZE: usize = 80;
    const TLS_DIRECT_CAPTURE_EVENT_SIZE: usize = 88 + 4_194_304;
    const FILE_EVENT_HEADER_SIZE: usize = 128;
    const FILE_EVENT_PRIMARY_PATH_SIZE: usize = FILE_EVENT_HEADER_SIZE + 256;
    const FILE_EVENT_SIZE: usize = FILE_EVENT_HEADER_SIZE + 256 * 2;
    const STDIO_EVENT_SIZE: usize = 80 + 4_096;
    const STDIO_COMPLETION_EVENT_SIZE: usize = 88;
    const SOCKET_EVENT_SIZE: usize = 80 + 4_096;
    const SOCKET_COMPLETION_EVENT_SIZE: usize = 96;

    if !known_event_kind(kind) {
        return false;
    }

    match kind {
        1 => size == PROCESS_FORK_EVENT_SIZE,
        2 => size == PROCESS_EXEC_EVENT_SIZE,
        3 => size == PROCESS_EXIT_EVENT_SIZE,
        4 => size == PROCESS_SIGNAL_EVENT_SIZE,
        100 | 101 | 104..=107 => size == NETWORK_EVENT_SIZE,
        102 | 103 => size == FD_IO_EVENT_SIZE,
        108 => size == SOCKET_RELEASE_EVENT_SIZE,
        201 | 202 => size == TLS_PAYLOAD_FIXED_EVENT_SIZE,
        203 => size == TLS_DIRECT_CAPTURE_EVENT_SIZE,
        204 => size == TLS_DIAGNOSTIC_EVENT_SIZE,
        205 => size == LAUNCH_BINDING_FAILURE_EVENT_SIZE,
        300..=308 => {
            matches!(
                size,
                FILE_EVENT_HEADER_SIZE | FILE_EVENT_PRIMARY_PATH_SIZE | FILE_EVENT_SIZE
            )
        }
        400 => size == STDIO_EVENT_SIZE,
        401 => size == STDIO_COMPLETION_EVENT_SIZE,
        500 => size == SOCKET_EVENT_SIZE,
        501 => size == SOCKET_COMPLETION_EVENT_SIZE,
        _ => false,
    }
}

#[cfg(any(feature = "perf-buffer", actrail_event_transport_perf))]
fn known_event_kind(kind: u32) -> bool {
    matches!(
        kind,
        1..=4 | 100..=108 | 201..=205 | 300..=308 | 400 | 401 | 500 | 501
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

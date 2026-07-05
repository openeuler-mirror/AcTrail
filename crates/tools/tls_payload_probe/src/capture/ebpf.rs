//! libbpf runtime for TLS payload uprobes.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::Path;
use std::rc::Rc;
#[cfg(feature = "perf-buffer")]
use std::sync::Arc;
#[cfg(feature = "perf-buffer")]
use std::sync::atomic::{AtomicU64, Ordering};

use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder, UprobeOpts};
#[cfg(feature = "perf-buffer")]
use libbpf_rs::{PerfBuffer, PerfBufferBuilder};
#[cfg(not(feature = "perf-buffer"))]
use libbpf_rs::{RingBuffer, RingBufferBuilder};
use tls_probe_point_finder::{
    CaptureStrategy, PayloadDirection, ProbePoint, ProbePointPlan, TlsProvider,
};

use crate::capture::config::CaptureConfig;
use crate::capture::event::{CaptureDirection, CaptureEvent, CaptureFlags};
use crate::capture::ring_stats::RingLostStats;
use crate::capture::segment::{PayloadSegment, PayloadSegmentAssembler};
use crate::{ToolError, ToolResult};

const EVENT_KIND_PAYLOAD: u32 = 1;
const EVENT_FLAG_TRUNCATED: u32 = 1;
const EVENT_FLAG_RUSTLS_CHUNK: u32 = 2;
const HEADER_BYTES: usize = 72;

const PROVIDER_OPENSSL: u32 = 1;
const PROVIDER_BORINGSSL: u32 = 2;
const PROVIDER_RUSTLS: u32 = 3;
const PROVIDER_GO: u32 = 4;
const PROVIDER_GNUTLS: u32 = 5;
const PROVIDER_NSS: u32 = 6;

const DIRECTION_OUTBOUND: u32 = 1;
const DIRECTION_INBOUND: u32 = 2;

const SYMBOL_SSL_WRITE: u32 = 1;
const SYMBOL_SSL_READ: u32 = 2;
const SYMBOL_SSL_WRITE_EX: u32 = 3;
const SYMBOL_SSL_READ_EX: u32 = 4;
const SYMBOL_RUSTLS_BUFFER_PLAINTEXT: u32 = 5;
const SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT: u32 = 6;
const SYMBOL_GNUTLS_RECORD_SEND: u32 = 7;
const SYMBOL_GNUTLS_RECORD_RECV: u32 = 8;
const SYMBOL_NSPR_PR_WRITE: u32 = 9;
const SYMBOL_NSPR_PR_SEND: u32 = 10;
const SYMBOL_NSPR_PR_READ: u32 = 11;
const SYMBOL_NSPR_PR_RECV: u32 = 12;

const CONFIG_MAP_BYTES: usize = std::mem::size_of::<u32>() * 4;
const RING_DIAGNOSTICS_KEY: u32 = 0;
const RING_DIAGNOSTIC_WORDS: usize = 9;
const RING_DIAGNOSTICS_BYTES: usize = std::mem::size_of::<u64>() * RING_DIAGNOSTIC_WORDS;
const RING_DIAG_RESERVE_FAIL_EVENTS_INDEX: usize = 0;
const RING_DIAG_RESERVE_FAIL_ACTUAL_BYTES_INDEX: usize = 1;
const RING_DIAG_RESERVE_FAIL_RESERVED_BYTES_INDEX: usize = 2;
const RING_DIAG_READ_USER_FAIL_EVENTS_INDEX: usize = 3;
const RING_DIAG_READ_USER_FAIL_ACTUAL_BYTES_INDEX: usize = 4;
const RING_DIAG_READ_USER_FAIL_RESERVED_BYTES_INDEX: usize = 5;
const RING_DIAG_OUTPUT_FAIL_EVENTS_INDEX: usize = 6;
const RING_DIAG_OUTPUT_FAIL_ACTUAL_BYTES_INDEX: usize = 7;
const RING_DIAG_OUTPUT_FAIL_RESERVED_BYTES_INDEX: usize = 8;

pub(super) struct BpfPayloadRuntime {
    _object: Object,
    _links: Vec<Link>,
    ring_diagnostics: MapHandle,
    events: Rc<RefCell<Vec<Vec<u8>>>>,
    segments: PayloadSegmentAssembler,
    event_buffer: EventBuffer,
}

impl BpfPayloadRuntime {
    pub(super) fn load(
        config: &CaptureConfig,
        plan: &ProbePointPlan,
        target_pid: u32,
    ) -> ToolResult<Self> {
        let object_bytes = include_bytes!(env!("TLS_PAYLOAD_PROBE_BPF_OBJECT"));
        let mut builder = ObjectBuilder::default();
        let mut open_object = builder
            .open_memory(object_bytes)
            .map_err(|error| ToolError::new(format!("open BPF object: {error}")))?;
        resize_map(
            &mut open_object,
            "events",
            event_map_max_entries(config.ring_buffer_bytes)?,
        )?;
        resize_map(&mut open_object, "pending_ops", config.pending_ops)?;
        let mut object = open_object
            .load()
            .map_err(|error| ToolError::new(format!("load BPF object: {error}")))?;
        configure_probe_config(&object, config, plan.provider)?;
        let events_map = map_handle(&object, "events")?;
        let ring_diagnostics = map_handle(&object, "ring_diagnostics")?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let event_buffer =
            EventBuffer::build(&events_map, Rc::clone(&events), config.ring_buffer_bytes)?;
        let links = attach_plan(&mut object, plan, target_pid)?;
        if links.is_empty() {
            return Err(ToolError::new("BPF runtime did not attach any uprobes"));
        }
        Ok(Self {
            _object: object,
            _links: links,
            ring_diagnostics,
            events,
            segments: PayloadSegmentAssembler::default(),
            event_buffer,
        })
    }

    pub(super) fn poll_events(&mut self) -> ToolResult<Vec<CaptureEvent>> {
        self.event_buffer.consume()?;
        let mut events = Vec::new();
        for raw in std::mem::take(&mut *self.events.borrow_mut()) {
            events.extend(self.segments.push(decode_segment(&raw)?)?);
        }
        Ok(events)
    }

    pub(super) fn finish_events(&mut self) -> ToolResult<Vec<CaptureEvent>> {
        self.segments.finish()
    }

    pub(super) fn ring_lost_stats(&self) -> ToolResult<RingLostStats> {
        let key = RING_DIAGNOSTICS_KEY.to_ne_bytes();
        let value = self
            .ring_diagnostics
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| ToolError::new(format!("read ring diagnostics: {error}")))?
            .ok_or_else(|| ToolError::new("missing ring diagnostics record"))?;
        let words = read_u64_words(&value)?;
        Ok(RingLostStats {
            reserve_fail_events: words[RING_DIAG_RESERVE_FAIL_EVENTS_INDEX],
            reserve_fail_actual_bytes: words[RING_DIAG_RESERVE_FAIL_ACTUAL_BYTES_INDEX],
            reserve_fail_reserved_bytes: words[RING_DIAG_RESERVE_FAIL_RESERVED_BYTES_INDEX],
            read_user_fail_events: words[RING_DIAG_READ_USER_FAIL_EVENTS_INDEX],
            read_user_fail_actual_bytes: words[RING_DIAG_READ_USER_FAIL_ACTUAL_BYTES_INDEX],
            read_user_fail_reserved_bytes: words[RING_DIAG_READ_USER_FAIL_RESERVED_BYTES_INDEX],
            output_fail_events: words[RING_DIAG_OUTPUT_FAIL_EVENTS_INDEX],
            output_fail_actual_bytes: words[RING_DIAG_OUTPUT_FAIL_ACTUAL_BYTES_INDEX],
            output_fail_reserved_bytes: words[RING_DIAG_OUTPUT_FAIL_RESERVED_BYTES_INDEX],
            perf_lost_events: self.event_buffer.lost_count(),
        })
    }
}

enum EventBuffer {
    #[cfg(not(feature = "perf-buffer"))]
    Ring(RingBuffer<'static>),
    #[cfg(feature = "perf-buffer")]
    Perf {
        buffer: PerfBuffer<'static>,
        lost: Arc<AtomicU64>,
    },
}

impl EventBuffer {
    fn build(
        events_map: &MapHandle,
        events: Rc<RefCell<Vec<Vec<u8>>>>,
        buffer_bytes: u32,
    ) -> ToolResult<Self> {
        #[cfg(feature = "perf-buffer")]
        {
            let callback_events = Rc::clone(&events);
            let lost = Arc::new(AtomicU64::new(0));
            let callback_lost = Arc::clone(&lost);
            let buffer = PerfBufferBuilder::new(events_map)
                .sample_cb(move |_cpu, raw| {
                    callback_events
                        .borrow_mut()
                        .push(perf_sample_payload(raw).to_vec());
                })
                .lost_cb(move |_cpu, count| {
                    callback_lost.fetch_add(count, Ordering::Relaxed);
                })
                .pages(perf_pages_for_bytes(buffer_bytes)?)
                .build()
                .map_err(|error| ToolError::new(format!("build perfbuf: {error}")))?;
            Ok(Self::Perf { buffer, lost })
        }
        #[cfg(not(feature = "perf-buffer"))]
        {
            let _ = buffer_bytes;
            let callback_events = Rc::clone(&events);
            let mut builder = RingBufferBuilder::new();
            builder
                .add(events_map, move |raw| {
                    callback_events.borrow_mut().push(raw.to_vec());
                    0
                })
                .map_err(|error| ToolError::new(format!("create ringbuf callback: {error}")))?;
            let buffer = builder
                .build()
                .map_err(|error| ToolError::new(format!("build ringbuf: {error}")))?;
            Ok(Self::Ring(buffer))
        }
    }

    fn consume(&self) -> ToolResult<()> {
        match self {
            #[cfg(not(feature = "perf-buffer"))]
            Self::Ring(buffer) => buffer
                .consume()
                .map_err(|error| ToolError::new(format!("consume ringbuf: {error}"))),
            #[cfg(feature = "perf-buffer")]
            Self::Perf { buffer, .. } => buffer
                .consume()
                .map_err(|error| ToolError::new(format!("consume perfbuf: {error}"))),
        }
    }

    fn lost_count(&self) -> u64 {
        match self {
            #[cfg(not(feature = "perf-buffer"))]
            Self::Ring(_) => 0,
            #[cfg(feature = "perf-buffer")]
            Self::Perf { lost, .. } => lost.load(Ordering::Relaxed),
        }
    }
}

fn resize_map(
    open_object: &mut libbpf_rs::OpenObject,
    map_name: &str,
    max_entries: u32,
) -> ToolResult<()> {
    let mut map = open_object
        .maps_mut()
        .find(|map| map.name() == OsStr::new(map_name))
        .ok_or_else(|| ToolError::new(format!("BPF map {map_name} is missing")))?;
    map.set_max_entries(max_entries)
        .map_err(|error| ToolError::new(format!("resize BPF map {map_name}: {error}")))
}

#[cfg(feature = "perf-buffer")]
fn event_map_max_entries(_buffer_bytes: u32) -> ToolResult<u32> {
    let cpus = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_CONF) };
    if cpus <= 0 {
        return Err(ToolError::new(format!(
            "invalid configured CPU count {cpus}"
        )));
    }
    u32::try_from(cpus).map_err(|error| ToolError::new(format!("CPU count overflow: {error}")))
}

#[cfg(not(feature = "perf-buffer"))]
fn event_map_max_entries(buffer_bytes: u32) -> ToolResult<u32> {
    Ok(buffer_bytes)
}

#[cfg(feature = "perf-buffer")]
fn perf_pages_for_bytes(buffer_bytes: u32) -> ToolResult<usize> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(ToolError::new(format!(
            "invalid system page size {page_size}"
        )));
    }
    let page_size = usize::try_from(page_size)
        .map_err(|error| ToolError::new(format!("page size overflow: {error}")))?;
    let bytes = usize::try_from(buffer_bytes)
        .map_err(|error| ToolError::new(format!("buffer size overflow: {error}")))?;
    Ok(bytes.div_ceil(page_size).max(1).next_power_of_two())
}

#[cfg(feature = "perf-buffer")]
fn perf_sample_payload(raw: &[u8]) -> &[u8] {
    strip_perf_raw_size_prefix(raw)
        .or_else(|| strip_perf_trailing_padding(raw))
        .unwrap_or(raw)
}

#[cfg(feature = "perf-buffer")]
fn strip_perf_raw_size_prefix(raw: &[u8]) -> Option<&[u8]> {
    let declared_size = read_u32_optional(raw, 0)? as usize;
    let payload = raw.get(4..)?;
    let event = payload.get(..declared_size)?;
    if payload_event_size(event)? == declared_size {
        Some(event)
    } else {
        None
    }
}

#[cfg(feature = "perf-buffer")]
fn strip_perf_trailing_padding(raw: &[u8]) -> Option<&[u8]> {
    let size = payload_event_size(raw)?;
    raw.get(..size)
}

#[cfg(feature = "perf-buffer")]
fn payload_event_size(raw: &[u8]) -> Option<usize> {
    if read_u32_optional(raw, 0)? != EVENT_KIND_PAYLOAD {
        return None;
    }
    let captured_size = read_u32_optional(raw, 28)? as usize;
    let size = HEADER_BYTES.checked_add(captured_size)?;
    if size <= raw.len() { Some(size) } else { None }
}

#[cfg(feature = "perf-buffer")]
fn read_u32_optional(raw: &[u8], offset: usize) -> Option<u32> {
    raw.get(offset..offset + std::mem::size_of::<u32>())
        .and_then(|value| value.try_into().ok())
        .map(u32::from_ne_bytes)
}

fn configure_probe_config(
    object: &Object,
    config: &CaptureConfig,
    provider: TlsProvider,
) -> ToolResult<()> {
    let map = map_handle(object, "probe_config")?;
    let key = 0_u32.to_ne_bytes();
    let mut value = [0_u8; CONFIG_MAP_BYTES];
    value[0..4].copy_from_slice(&(config.max_capture_bytes as u32).to_ne_bytes());
    value[4..8].copy_from_slice(&provider_id(provider).to_ne_bytes());
    value[8..12].copy_from_slice(&config.rustls_chunks.to_ne_bytes());
    map.update(&key, &value, MapFlags::ANY)
        .map_err(|error| ToolError::new(format!("update probe_config map: {error}")))
}

fn map_handle(object: &Object, map_name: &str) -> ToolResult<MapHandle> {
    object
        .maps()
        .find(|map| map.name() == OsStr::new(map_name))
        .ok_or_else(|| ToolError::new(format!("BPF map {map_name} is missing")))
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| ToolError::new(format!("open BPF map {map_name}: {error}")))
        })
}

fn attach_plan(
    object: &mut Object,
    plan: &ProbePointPlan,
    target_pid: u32,
) -> ToolResult<Vec<Link>> {
    let pid = i32::try_from(target_pid)
        .map_err(|error| ToolError::new(format!("target pid overflow: {error}")))?;
    let mut links = Vec::new();
    for point in &plan.points {
        if point.symbol == "SSL_write_ex2" {
            continue;
        }
        if point.direction == PayloadDirection::Control {
            continue;
        }
        match point.capture {
            CaptureStrategy::EntryBuffer => {
                links.push(attach_program(
                    object,
                    entry_program(point)?,
                    &plan.binary.path,
                    point.file_offset,
                    pid,
                    false,
                )?);
            }
            CaptureStrategy::ReturnBufferFromEntryState => {
                links.push(attach_program(
                    object,
                    return_entry_program(point)?,
                    &plan.binary.path,
                    point.file_offset,
                    pid,
                    false,
                )?);
                links.push(attach_program(
                    object,
                    return_program(point)?,
                    &plan.binary.path,
                    point.file_offset,
                    pid,
                    true,
                )?);
            }
        }
    }
    Ok(links)
}

fn attach_program(
    object: &mut Object,
    program_name: &str,
    path: &Path,
    offset: u64,
    pid: i32,
    retprobe: bool,
) -> ToolResult<Link> {
    let offset = usize::try_from(offset)
        .map_err(|error| ToolError::new(format!("uprobe offset overflow: {error}")))?;
    let program = object
        .progs_mut()
        .find(|program| program.name() == OsStr::new(program_name))
        .ok_or_else(|| ToolError::new(format!("BPF program {program_name} is missing")))?;
    program
        .attach_uprobe_with_opts(
            pid,
            path,
            offset,
            UprobeOpts {
                retprobe,
                ..Default::default()
            },
        )
        .map_err(|error| {
            ToolError::new(format!(
                "attach {program_name} to {}:0x{offset:x}: {error}",
                path.display()
            ))
        })
}

fn entry_program(point: &ProbePoint) -> ToolResult<&'static str> {
    match point.symbol.as_str() {
        "SSL_write" => Ok("handle_ssl_write"),
        "SSL_write_ex" => Ok("handle_ssl_write_ex"),
        "rustls_buffer_plaintext" => Ok("handle_rustls_buffer_plaintext"),
        "rustls_take_received_plaintext" => Ok("handle_rustls_take_received_plaintext"),
        symbol => Err(ToolError::new(format!(
            "unsupported entry payload symbol: {symbol}"
        ))),
    }
}

fn return_entry_program(point: &ProbePoint) -> ToolResult<&'static str> {
    match point.symbol.as_str() {
        "SSL_read" | "SSL_read_internal" => Ok("handle_ssl_read_enter"),
        "SSL_read_ex" => Ok("handle_ssl_read_ex_enter"),
        "gnutls_record_send" => Ok("handle_gnutls_record_send_enter"),
        "gnutls_record_recv" => Ok("handle_gnutls_record_recv_enter"),
        "PR_Write" => Ok("handle_nspr_pr_write_enter"),
        "PR_Send" => Ok("handle_nspr_pr_send_enter"),
        "PR_Read" => Ok("handle_nspr_pr_read_enter"),
        "PR_Recv" => Ok("handle_nspr_pr_recv_enter"),
        symbol => Err(ToolError::new(format!(
            "unsupported return-entry payload symbol: {symbol}"
        ))),
    }
}

fn return_program(point: &ProbePoint) -> ToolResult<&'static str> {
    match point.symbol.as_str() {
        "SSL_read" | "SSL_read_internal" => Ok("handle_ssl_read_return"),
        "SSL_read_ex" => Ok("handle_ssl_read_ex_return"),
        "gnutls_record_send" => Ok("handle_gnutls_record_send_return"),
        "gnutls_record_recv" => Ok("handle_gnutls_record_recv_return"),
        "PR_Write" => Ok("handle_nspr_pr_write_return"),
        "PR_Send" => Ok("handle_nspr_pr_send_return"),
        "PR_Read" => Ok("handle_nspr_pr_read_return"),
        "PR_Recv" => Ok("handle_nspr_pr_recv_return"),
        symbol => Err(ToolError::new(format!(
            "unsupported return payload symbol: {symbol}"
        ))),
    }
}

fn decode_segment(raw: &[u8]) -> ToolResult<PayloadSegment> {
    if raw.len() < HEADER_BYTES {
        return Err(ToolError::new(format!(
            "short BPF payload event: {} bytes",
            raw.len()
        )));
    }
    let kind = read_u32(raw, 0)?;
    if kind != EVENT_KIND_PAYLOAD {
        return Err(ToolError::new(format!("unknown BPF event kind: {kind}")));
    }
    let captured_size = read_u32(raw, 28)? as usize;
    let payload_end = HEADER_BYTES
        .checked_add(captured_size)
        .ok_or_else(|| ToolError::new("BPF payload event size overflow"))?;
    if payload_end > raw.len() {
        return Err(ToolError::new(format!(
            "BPF payload event declared {captured_size} bytes, raw size is {}",
            raw.len()
        )));
    }
    let flags = read_u32(raw, 24)?;
    Ok(PayloadSegment {
        pid: read_u32(raw, 4)?,
        tid: read_u32(raw, 8)?,
        provider: provider_name(read_u32(raw, 16)?).to_string(),
        symbol: symbol_name(read_u32(raw, 20)?).to_string(),
        direction: direction(read_u32(raw, 12)?)?,
        requested_size: read_u64(raw, 32)?,
        observed_ktime_ns: read_u64(raw, 40)?,
        stream_key: read_u64(raw, 48)?,
        segment_offset: read_u64(raw, 56)?,
        operation_size: read_u64(raw, 64)?,
        flags: CaptureFlags {
            truncated: flags & EVENT_FLAG_TRUNCATED != 0,
            rustls_chunk: flags & EVENT_FLAG_RUSTLS_CHUNK != 0,
        },
        captured: raw[HEADER_BYTES..payload_end].to_vec(),
    })
}

fn read_u32(raw: &[u8], offset: usize) -> ToolResult<u32> {
    let bytes = raw
        .get(offset..offset + std::mem::size_of::<u32>())
        .ok_or_else(|| ToolError::new("short BPF u32 field"))?;
    Ok(u32::from_ne_bytes(
        bytes.try_into().expect("u32 field length checked"),
    ))
}

fn read_u64(raw: &[u8], offset: usize) -> ToolResult<u64> {
    let bytes = raw
        .get(offset..offset + std::mem::size_of::<u64>())
        .ok_or_else(|| ToolError::new("short BPF u64 field"))?;
    Ok(u64::from_ne_bytes(
        bytes.try_into().expect("u64 field length checked"),
    ))
}

fn read_u64_words(raw: &[u8]) -> ToolResult<[u64; RING_DIAGNOSTIC_WORDS]> {
    let raw = raw
        .get(..RING_DIAGNOSTICS_BYTES)
        .ok_or_else(|| ToolError::new("short BPF ring diagnostics value"))?;
    let mut words = [0_u64; RING_DIAGNOSTIC_WORDS];
    for (index, word) in words.iter_mut().enumerate() {
        let offset = index * std::mem::size_of::<u64>();
        let bytes = raw
            .get(offset..offset + std::mem::size_of::<u64>())
            .expect("ring diagnostics slice length checked");
        *word = u64::from_ne_bytes(
            bytes
                .try_into()
                .expect("ring diagnostics field length checked"),
        );
    }
    Ok(words)
}

fn provider_id(provider: TlsProvider) -> u32 {
    match provider {
        TlsProvider::OpenSsl => PROVIDER_OPENSSL,
        TlsProvider::BoringSsl => PROVIDER_BORINGSSL,
        TlsProvider::Rustls => PROVIDER_RUSTLS,
        TlsProvider::Go => PROVIDER_GO,
        TlsProvider::GnuTls => PROVIDER_GNUTLS,
        TlsProvider::Nss => PROVIDER_NSS,
    }
}

fn provider_name(provider: u32) -> &'static str {
    match provider {
        PROVIDER_OPENSSL => "openssl",
        PROVIDER_BORINGSSL => "boringssl",
        PROVIDER_RUSTLS => "rustls",
        PROVIDER_GO => "go",
        PROVIDER_GNUTLS => "gnutls",
        PROVIDER_NSS => "nss",
        _ => "unknown",
    }
}

fn symbol_name(symbol: u32) -> &'static str {
    match symbol {
        SYMBOL_SSL_WRITE => "SSL_write",
        SYMBOL_SSL_READ => "SSL_read",
        SYMBOL_SSL_WRITE_EX => "SSL_write_ex",
        SYMBOL_SSL_READ_EX => "SSL_read_ex",
        SYMBOL_RUSTLS_BUFFER_PLAINTEXT => "rustls_buffer_plaintext",
        SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT => "rustls_take_received_plaintext",
        SYMBOL_GNUTLS_RECORD_SEND => "gnutls_record_send",
        SYMBOL_GNUTLS_RECORD_RECV => "gnutls_record_recv",
        SYMBOL_NSPR_PR_WRITE => "PR_Write",
        SYMBOL_NSPR_PR_SEND => "PR_Send",
        SYMBOL_NSPR_PR_READ => "PR_Read",
        SYMBOL_NSPR_PR_RECV => "PR_Recv",
        _ => "unknown",
    }
}

fn direction(direction: u32) -> ToolResult<CaptureDirection> {
    match direction {
        DIRECTION_INBOUND => Ok(CaptureDirection::Inbound),
        DIRECTION_OUTBOUND => Ok(CaptureDirection::Outbound),
        _ => Err(ToolError::new(format!(
            "unknown BPF payload direction: {direction}"
        ))),
    }
}

#[cfg(all(test, feature = "perf-buffer"))]
mod tests {
    use super::{HEADER_BYTES, perf_sample_payload};

    #[test]
    fn perf_sample_payload_strips_raw_size_prefix_with_padding() {
        let event = payload_event(3);
        let mut raw = Vec::new();
        raw.extend_from_slice(&(event.len() as u32).to_ne_bytes());
        raw.extend_from_slice(&event);
        raw.extend_from_slice(&[0, 0, 0, 0]);

        let payload = perf_sample_payload(&raw);

        assert_eq!(payload, event.as_slice());
    }

    #[test]
    fn perf_sample_payload_strips_trailing_padding() {
        let event = payload_event(5);
        let mut raw = event.clone();
        raw.extend_from_slice(&[0, 0, 0]);

        let payload = perf_sample_payload(&raw);

        assert_eq!(payload, event.as_slice());
    }

    fn payload_event(captured_size: u32) -> Vec<u8> {
        let mut event = vec![0; HEADER_BYTES + captured_size as usize];
        event[0..4].copy_from_slice(&1_u32.to_ne_bytes());
        event[28..32].copy_from_slice(&captured_size.to_ne_bytes());
        event
    }
}

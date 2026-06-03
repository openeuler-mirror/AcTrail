//! libbpf runtime for TLS payload uprobes.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::Path;
use std::rc::Rc;

use libbpf_rs::{
    Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder, RingBuffer, RingBufferBuilder,
    UprobeOpts,
};
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

const DIRECTION_OUTBOUND: u32 = 1;
const DIRECTION_INBOUND: u32 = 2;

const SYMBOL_SSL_WRITE: u32 = 1;
const SYMBOL_SSL_READ: u32 = 2;
const SYMBOL_SSL_WRITE_EX: u32 = 3;
const SYMBOL_SSL_READ_EX: u32 = 4;
const SYMBOL_RUSTLS_BUFFER_PLAINTEXT: u32 = 5;
const SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT: u32 = 6;

const CONFIG_MAP_BYTES: usize = std::mem::size_of::<u32>() * 4;
const RING_DIAGNOSTICS_KEY: u32 = 0;
const RING_DIAGNOSTIC_WORDS: usize = 6;
const RING_DIAGNOSTICS_BYTES: usize = std::mem::size_of::<u64>() * RING_DIAGNOSTIC_WORDS;
const RING_DIAG_RESERVE_FAIL_EVENTS_INDEX: usize = 0;
const RING_DIAG_RESERVE_FAIL_ACTUAL_BYTES_INDEX: usize = 1;
const RING_DIAG_RESERVE_FAIL_RESERVED_BYTES_INDEX: usize = 2;
const RING_DIAG_READ_USER_FAIL_EVENTS_INDEX: usize = 3;
const RING_DIAG_READ_USER_FAIL_ACTUAL_BYTES_INDEX: usize = 4;
const RING_DIAG_READ_USER_FAIL_RESERVED_BYTES_INDEX: usize = 5;

pub(super) struct BpfPayloadRuntime {
    _object: Object,
    _links: Vec<Link>,
    ring_diagnostics: MapHandle,
    events: Rc<RefCell<Vec<Vec<u8>>>>,
    segments: PayloadSegmentAssembler,
    ring_buffer: RingBuffer<'static>,
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
        resize_map(&mut open_object, "events", config.ring_buffer_bytes)?;
        resize_map(&mut open_object, "pending_ops", config.pending_ops)?;
        let mut object = open_object
            .load()
            .map_err(|error| ToolError::new(format!("load BPF object: {error}")))?;
        configure_probe_config(&object, config, plan.provider)?;
        let events_map = map_handle(&object, "events")?;
        let ring_diagnostics = map_handle(&object, "ring_diagnostics")?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let callback_events = Rc::clone(&events);
        let mut ring_buffer_builder = RingBufferBuilder::new();
        ring_buffer_builder
            .add(&events_map, move |raw| {
                callback_events.borrow_mut().push(raw.to_vec());
                0
            })
            .map_err(|error| ToolError::new(format!("create ringbuf callback: {error}")))?;
        let ring_buffer = ring_buffer_builder
            .build()
            .map_err(|error| ToolError::new(format!("build ringbuf: {error}")))?;
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
            ring_buffer,
        })
    }

    pub(super) fn poll_events(&mut self) -> ToolResult<Vec<CaptureEvent>> {
        self.ring_buffer
            .consume()
            .map_err(|error| ToolError::new(format!("consume ringbuf: {error}")))?;
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
        })
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
        symbol => Err(ToolError::new(format!(
            "unsupported return-entry payload symbol: {symbol}"
        ))),
    }
}

fn return_program(point: &ProbePoint) -> ToolResult<&'static str> {
    match point.symbol.as_str() {
        "SSL_read" | "SSL_read_internal" => Ok("handle_ssl_read_return"),
        "SSL_read_ex" => Ok("handle_ssl_read_ex_return"),
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
    }
}

fn provider_name(provider: u32) -> &'static str {
    match provider {
        PROVIDER_OPENSSL => "openssl",
        PROVIDER_BORINGSSL => "boringssl",
        PROVIDER_RUSTLS => "rustls",
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

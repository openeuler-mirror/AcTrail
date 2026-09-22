//! Hand-written compact binary codec for event payloads.
//!
//! Replaces bincode with a fixed, minimal wire format: a `u8` variant tag
//! (bincode uses `u32`), LEB128 length prefixes, and a static metadata-key
//! dictionary so the common short keys (`operation`, `direction`, `fd`, ...)
//! store as one byte instead of repeating the key name per event.

use std::collections::BTreeMap;

use model_core::event::{
    ApplicationBody, ApplicationPayload, ControlPayload, EnforcementPayload, EventPayload,
    FilePayload, IpcPayload, LabelPayload, LossPayload, MemoryEventCounters, NetPayload,
    ProcessPayload, ResourceAccountingCoverage, ResourceAccountingMethod, ResourcePayload,
    ResourceSampleKind, StdioPayload,
};

use super::key_codes::MetadataKeyCodebook;
use super::value_codes::{value_code, value_for_code};
use model_core::process::ProcessIdentity;

use super::EventPayloadCodec;

const UNKNOWN_KEY: u8 = 0xFF;
const RESOURCE_LEGACY_TAG: u8 = 6;
const RESOURCE_V2_TAG: u8 = 11;

#[path = "manual_file_summary.rs"]
mod file_summary;

pub struct ManualCodec;

impl EventPayloadCodec for ManualCodec {
    fn encode(&self, payload: &EventPayload) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(256);
        match payload {
            EventPayload::Process(p) => {
                out.push(0);
                encode_process(&mut out, p);
            }
            EventPayload::File(p) => {
                out.push(1);
                encode_file(&mut out, p);
            }
            EventPayload::Net(p) => {
                out.push(2);
                encode_net(&mut out, p);
            }
            EventPayload::Ipc(p) => {
                out.push(3);
                encode_ipc(&mut out, p);
            }
            EventPayload::Stdio(p) => {
                out.push(4);
                encode_stdio(&mut out, p);
            }
            EventPayload::Application(p) => {
                out.push(5);
                encode_application(&mut out, p);
            }
            EventPayload::Resource(p) => {
                out.push(RESOURCE_V2_TAG);
                encode_resource_v2(&mut out, p);
            }
            EventPayload::Control(p) => {
                out.push(7);
                encode_control(&mut out, p);
            }
            EventPayload::Loss(p) => {
                out.push(8);
                encode_loss(&mut out, p);
            }
            EventPayload::Label(p) => {
                out.push(9);
                encode_label(&mut out, p);
            }
            EventPayload::Enforcement(p) => {
                out.push(10);
                encode_enforcement(&mut out, p);
            }
        }
        Ok(out)
    }

    fn decode(&self, bytes: &[u8]) -> Result<EventPayload, String> {
        let mut cursor = 0usize;
        let variant = read_u8(bytes, &mut cursor)?;
        let payload = match variant {
            0 => EventPayload::Process(decode_process(bytes, &mut cursor)?),
            1 => EventPayload::File(decode_file(bytes, &mut cursor)?),
            2 => EventPayload::Net(decode_net(bytes, &mut cursor)?),
            3 => EventPayload::Ipc(decode_ipc(bytes, &mut cursor)?),
            4 => EventPayload::Stdio(decode_stdio(bytes, &mut cursor)?),
            5 => EventPayload::Application(decode_application(bytes, &mut cursor)?),
            RESOURCE_LEGACY_TAG => {
                EventPayload::Resource(decode_resource_legacy(bytes, &mut cursor)?)
            }
            7 => EventPayload::Control(decode_control(bytes, &mut cursor)?),
            8 => EventPayload::Loss(decode_loss(bytes, &mut cursor)?),
            9 => EventPayload::Label(decode_label(bytes, &mut cursor)?),
            10 => EventPayload::Enforcement(decode_enforcement(bytes, &mut cursor)?),
            RESOURCE_V2_TAG => EventPayload::Resource(decode_resource_v2(bytes, &mut cursor)?),
            other => return Err(format!("unknown event payload variant {other}")),
        };
        if cursor != bytes.len() {
            return Err("trailing bytes after event payload".to_string());
        }
        Ok(payload)
    }
}

// ---- encoders ----

fn encode_process(out: &mut Vec<u8>, p: &ProcessPayload) {
    write_string(out, &p.operation);
    write_option_u64(out, p.parent.map(ProcessIdentity::get));
    write_option_string(out, &p.executable);
    write_map(out, &p.metadata);
}

fn encode_file(out: &mut Vec<u8>, p: &FilePayload) {
    write_string(out, &p.operation);
    write_option_string(out, &p.path);
    write_option_i32(out, p.result);
    write_map(out, &p.metadata);
    file_summary::encode(out, &p.io_summary);
}

fn encode_net(out: &mut Vec<u8>, p: &NetPayload) {
    write_string(out, &p.transport);
    write_option_string(out, &p.local);
    write_option_string(out, &p.remote);
    write_option_u64(out, p.size);
    write_option_i32(out, p.result);
    write_map(out, &p.metadata);
}

fn encode_ipc(out: &mut Vec<u8>, p: &IpcPayload) {
    write_string(out, &p.channel);
    write_option_string(out, &p.peer);
    write_option_u64(out, p.size);
    write_map(out, &p.metadata);
}

fn encode_stdio(out: &mut Vec<u8>, p: &StdioPayload) {
    write_string(out, &p.stream);
    write_bytes(out, &p.data);
    write_option_u64(out, p.original_size.map(|value| value as u64));
    write_bool(out, p.truncated);
}

fn encode_application(out: &mut Vec<u8>, p: &ApplicationPayload) {
    write_string(out, &p.protocol);
    write_string(out, &p.operation);
    write_string(out, &p.summary);
    write_option_body(out, &p.body);
    write_map(out, &p.metadata);
}

fn encode_resource_v2(out: &mut Vec<u8>, p: &ResourcePayload) {
    write_string(out, &p.scope);
    write_string(out, &p.subject);
    write_resource_accounting_method(out, p.accounting_method);
    write_resource_accounting_coverage(out, p.accounting_coverage);
    write_resource_sample_kind(out, p.sample_kind);
    write_option_u64(out, p.cpu_percent_millis);
    write_option_u64(out, p.rss_kb);
    write_option_u64(out, p.virtual_memory_kb);
    write_option_u64(out, p.memory_current_bytes);
    write_option_u64(out, p.memory_peak_bytes);
    write_option_u64(out, p.memory_anon_bytes);
    write_option_u64(out, p.memory_file_bytes);
    write_option_u64(out, p.memory_swap_current_bytes);
    write_option_memory_events(out, &p.memory_events);
    write_option_memory_events(out, &p.memory_events_local);
    write_option_u64(out, p.cpu_usage_usec);
    write_option_u64(out, p.cpu_user_usec);
    write_option_u64(out, p.cpu_system_usec);
    write_option_u64(out, p.cpu_nr_throttled);
    write_option_u64(out, p.cpu_throttled_usec);
    write_option_u64(out, p.io_read_bytes);
    write_option_u64(out, p.io_write_bytes);
    write_option_u64(out, p.pids_current);
    write_option_u64(out, p.pids_peak);
    write_option_u64(out, p.process_rss_sum_kb);
    write_map(out, &p.metadata);
}

fn encode_control(out: &mut Vec<u8>, p: &ControlPayload) {
    write_string(out, &p.action);
    write_string(out, &p.detail);
}

fn encode_loss(out: &mut Vec<u8>, p: &LossPayload) {
    write_string(out, &p.reason);
    write_bool(out, p.fatal);
}

fn encode_label(out: &mut Vec<u8>, p: &LabelPayload) {
    write_string(out, &p.provider);
    write_option_u16(out, p.confidence_millis);
    write_map(out, &p.evidence);
}

fn encode_enforcement(out: &mut Vec<u8>, p: &EnforcementPayload) {
    write_string(out, &p.backend);
    write_string(out, &p.operation);
    write_string(out, &p.decision);
    write_option_string(out, &p.path);
    write_option_string(out, &p.rule_id);
    write_string(out, &p.result);
    write_map(out, &p.metadata);
}

// ---- decoders ----

fn decode_process(bytes: &[u8], c: &mut usize) -> Result<ProcessPayload, String> {
    Ok(ProcessPayload {
        operation: read_string(bytes, c)?,
        parent: read_option_u64(bytes, c)?.map(ProcessIdentity::new),
        executable: read_option_string(bytes, c)?,
        metadata: read_map(bytes, c)?,
    })
}

fn decode_file(bytes: &[u8], c: &mut usize) -> Result<FilePayload, String> {
    Ok(FilePayload {
        operation: read_string(bytes, c)?,
        path: read_option_string(bytes, c)?,
        result: read_option_i32(bytes, c)?,
        metadata: read_map(bytes, c)?,
        io_summary: file_summary::decode(bytes, c)?,
    })
}

fn decode_net(bytes: &[u8], c: &mut usize) -> Result<NetPayload, String> {
    Ok(NetPayload {
        transport: read_string(bytes, c)?,
        local: read_option_string(bytes, c)?,
        remote: read_option_string(bytes, c)?,
        size: read_option_u64(bytes, c)?,
        result: read_option_i32(bytes, c)?,
        metadata: read_map(bytes, c)?,
    })
}

fn decode_ipc(bytes: &[u8], c: &mut usize) -> Result<IpcPayload, String> {
    Ok(IpcPayload {
        channel: read_string(bytes, c)?,
        peer: read_option_string(bytes, c)?,
        size: read_option_u64(bytes, c)?,
        metadata: read_map(bytes, c)?,
    })
}

fn decode_stdio(bytes: &[u8], c: &mut usize) -> Result<StdioPayload, String> {
    Ok(StdioPayload {
        stream: read_string(bytes, c)?,
        data: read_bytes(bytes, c)?,
        original_size: read_option_u64(bytes, c)?.map(|value| value as usize),
        truncated: read_bool(bytes, c)?,
    })
}

fn decode_application(bytes: &[u8], c: &mut usize) -> Result<ApplicationPayload, String> {
    Ok(ApplicationPayload {
        protocol: read_string(bytes, c)?,
        operation: read_string(bytes, c)?,
        summary: read_string(bytes, c)?,
        body: read_option_body(bytes, c)?,
        metadata: read_map(bytes, c)?,
    })
}

fn decode_resource_legacy(bytes: &[u8], c: &mut usize) -> Result<ResourcePayload, String> {
    let mut optimized_cursor = *c;
    if let Ok(payload) = decode_resource_legacy_optimized(bytes, &mut optimized_cursor) {
        if optimized_cursor == bytes.len() {
            *c = optimized_cursor;
            return Ok(payload);
        }
    }

    let mut pre_optimized_cursor = *c;
    let payload = decode_resource_legacy_pre_optimized(bytes, &mut pre_optimized_cursor)?;
    *c = pre_optimized_cursor;
    Ok(payload)
}

fn decode_resource_legacy_optimized(
    bytes: &[u8],
    c: &mut usize,
) -> Result<ResourcePayload, String> {
    Ok(ResourcePayload {
        scope: read_string(bytes, c)?,
        subject: read_string(bytes, c)?,
        cpu_percent_millis: read_option_u64(bytes, c)?,
        rss_kb: read_option_u64(bytes, c)?,
        virtual_memory_kb: read_option_u64(bytes, c)?,
        metadata: read_map(bytes, c)?,
        ..ResourcePayload::default()
    })
}

fn decode_resource_legacy_pre_optimized(
    bytes: &[u8],
    c: &mut usize,
) -> Result<ResourcePayload, String> {
    Ok(ResourcePayload {
        scope: read_legacy_string(bytes, c)?,
        subject: read_legacy_string(bytes, c)?,
        cpu_percent_millis: read_option_u64(bytes, c)?,
        rss_kb: read_option_u64(bytes, c)?,
        virtual_memory_kb: read_option_u64(bytes, c)?,
        metadata: read_legacy_map(bytes, c)?,
        ..ResourcePayload::default()
    })
}

fn decode_resource_v2(bytes: &[u8], c: &mut usize) -> Result<ResourcePayload, String> {
    Ok(ResourcePayload {
        scope: read_string(bytes, c)?,
        subject: read_string(bytes, c)?,
        accounting_method: read_resource_accounting_method(bytes, c)?,
        accounting_coverage: read_resource_accounting_coverage(bytes, c)?,
        sample_kind: read_resource_sample_kind(bytes, c)?,
        cpu_percent_millis: read_option_u64(bytes, c)?,
        rss_kb: read_option_u64(bytes, c)?,
        virtual_memory_kb: read_option_u64(bytes, c)?,
        memory_current_bytes: read_option_u64(bytes, c)?,
        memory_peak_bytes: read_option_u64(bytes, c)?,
        memory_anon_bytes: read_option_u64(bytes, c)?,
        memory_file_bytes: read_option_u64(bytes, c)?,
        memory_swap_current_bytes: read_option_u64(bytes, c)?,
        memory_events: read_option_memory_events(bytes, c)?,
        memory_events_local: read_option_memory_events(bytes, c)?,
        cpu_usage_usec: read_option_u64(bytes, c)?,
        cpu_user_usec: read_option_u64(bytes, c)?,
        cpu_system_usec: read_option_u64(bytes, c)?,
        cpu_nr_throttled: read_option_u64(bytes, c)?,
        cpu_throttled_usec: read_option_u64(bytes, c)?,
        io_read_bytes: read_option_u64(bytes, c)?,
        io_write_bytes: read_option_u64(bytes, c)?,
        pids_current: read_option_u64(bytes, c)?,
        pids_peak: read_option_u64(bytes, c)?,
        process_rss_sum_kb: read_option_u64(bytes, c)?,
        metadata: read_map(bytes, c)?,
    })
}

fn decode_control(bytes: &[u8], c: &mut usize) -> Result<ControlPayload, String> {
    Ok(ControlPayload {
        action: read_string(bytes, c)?,
        detail: read_string(bytes, c)?,
    })
}

fn decode_loss(bytes: &[u8], c: &mut usize) -> Result<LossPayload, String> {
    Ok(LossPayload {
        reason: read_string(bytes, c)?,
        fatal: read_bool(bytes, c)?,
    })
}

fn decode_label(bytes: &[u8], c: &mut usize) -> Result<LabelPayload, String> {
    Ok(LabelPayload {
        provider: read_string(bytes, c)?,
        confidence_millis: read_option_u16(bytes, c)?,
        evidence: read_map(bytes, c)?,
    })
}

fn decode_enforcement(bytes: &[u8], c: &mut usize) -> Result<EnforcementPayload, String> {
    Ok(EnforcementPayload {
        backend: read_string(bytes, c)?,
        operation: read_string(bytes, c)?,
        decision: read_string(bytes, c)?,
        path: read_option_string(bytes, c)?,
        rule_id: read_option_string(bytes, c)?,
        result: read_string(bytes, c)?,
        metadata: read_map(bytes, c)?,
    })
}

// ---- primitives ----

fn write_string(out: &mut Vec<u8>, value: &str) {
    if let Some(code) = value_code(value) {
        write_varint(out, (u64::from(code) << 1) | 1);
    } else {
        write_varint(out, (value.len() as u64) << 1);
        out.extend_from_slice(value.as_bytes());
    }
}

fn write_bytes(out: &mut Vec<u8>, value: &[u8]) {
    write_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn write_option_string(out: &mut Vec<u8>, value: &Option<String>) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            write_string(out, value);
        }
    }
}

fn write_option_u64(out: &mut Vec<u8>, value: Option<u64>) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn write_option_i32(out: &mut Vec<u8>, value: Option<i32>) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn write_option_u16(out: &mut Vec<u8>, value: Option<u16>) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn write_resource_accounting_method(out: &mut Vec<u8>, value: ResourceAccountingMethod) {
    out.push(match value {
        ResourceAccountingMethod::CgroupV2 => 0,
        ResourceAccountingMethod::ProcfsRssSum => 1,
    });
}

fn write_resource_accounting_coverage(out: &mut Vec<u8>, value: ResourceAccountingCoverage) {
    out.push(match value {
        ResourceAccountingCoverage::Exact => 0,
        ResourceAccountingCoverage::BroaderThanTrace => 1,
        ResourceAccountingCoverage::Partial => 2,
    });
}

fn write_resource_sample_kind(out: &mut Vec<u8>, value: ResourceSampleKind) {
    out.push(match value {
        ResourceSampleKind::Periodic => 0,
        ResourceSampleKind::Final => 1,
    });
}

fn write_option_memory_events(out: &mut Vec<u8>, value: &Option<MemoryEventCounters>) {
    let Some(value) = value else {
        out.push(0);
        return;
    };
    out.push(1);
    write_option_u64(out, value.low);
    write_option_u64(out, value.high);
    write_option_u64(out, value.max);
    write_option_u64(out, value.oom);
    write_option_u64(out, value.oom_kill);
    write_option_u64(out, value.oom_group_kill);
}

fn write_option_body(out: &mut Vec<u8>, value: &Option<ApplicationBody>) {
    match value {
        None => out.push(0),
        Some(ApplicationBody::Text(text)) => {
            out.push(1);
            write_string(out, text);
        }
        Some(ApplicationBody::Json(text)) => {
            out.push(2);
            write_string(out, text);
        }
        Some(ApplicationBody::Base64(text)) => {
            out.push(3);
            write_string(out, text);
        }
    }
}

fn write_bool(out: &mut Vec<u8>, value: bool) {
    out.push(u8::from(value));
}

fn write_map(out: &mut Vec<u8>, map: &BTreeMap<String, String>) {
    write_varint(out, map.len() as u64);
    for (key, value) in map {
        match MetadataKeyCodebook::code(key) {
            Some(code) => out.push(code),
            None => {
                out.push(UNKNOWN_KEY);
                write_string(out, key);
            }
        }
        write_string(out, value);
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn read_u8(bytes: &[u8], c: &mut usize) -> Result<u8, String> {
    let value = *bytes.get(*c).ok_or("unexpected end of payload")?;
    *c += 1;
    Ok(value)
}

fn read_bool(bytes: &[u8], c: &mut usize) -> Result<bool, String> {
    match read_u8(bytes, c)? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("invalid bool byte {other}")),
    }
}

fn read_varint(bytes: &[u8], c: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = read_u8(bytes, c)?;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 64 {
            return Err("varint overflow".to_string());
        }
    }
}

fn read_bytes(bytes: &[u8], c: &mut usize) -> Result<Vec<u8>, String> {
    let len = usize::try_from(read_varint(bytes, c)?).map_err(|_| "length overflow")?;
    let end = c.checked_add(len).ok_or("length overflow")?;
    let value = bytes.get(*c..end).ok_or("unexpected end of payload")?;
    *c = end;
    Ok(value.to_vec())
}

fn read_string(bytes: &[u8], c: &mut usize) -> Result<String, String> {
    let token = read_varint(bytes, c)?;
    if token & 1 == 1 {
        let code = u8::try_from(token >> 1).map_err(|_| "value code overflow")?;
        return value_for_code(code)
            .map(str::to_owned)
            .ok_or_else(|| format!("unknown payload value code {code}"));
    }
    let len = usize::try_from(token >> 1).map_err(|_| "length overflow")?;
    let end = c.checked_add(len).ok_or("length overflow")?;
    let value = bytes.get(*c..end).ok_or("unexpected end of payload")?;
    *c = end;
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| "invalid utf8 in payload".to_string())
}

fn read_legacy_string(bytes: &[u8], c: &mut usize) -> Result<String, String> {
    String::from_utf8(read_bytes(bytes, c)?).map_err(|_| "invalid utf8 in payload".to_string())
}

fn read_option_string(bytes: &[u8], c: &mut usize) -> Result<Option<String>, String> {
    match read_u8(bytes, c)? {
        0 => Ok(None),
        1 => Ok(Some(read_string(bytes, c)?)),
        other => Err(format!("invalid option tag {other}")),
    }
}

fn read_option_u64(bytes: &[u8], c: &mut usize) -> Result<Option<u64>, String> {
    match read_u8(bytes, c)? {
        0 => Ok(None),
        1 => {
            let end = c.checked_add(8).ok_or("length overflow")?;
            let raw: [u8; 8] = bytes
                .get(*c..end)
                .ok_or("unexpected end of payload")?
                .try_into()
                .map_err(|_| "unexpected end of payload")?;
            *c = end;
            Ok(Some(u64::from_le_bytes(raw)))
        }
        other => Err(format!("invalid option tag {other}")),
    }
}

fn read_option_i32(bytes: &[u8], c: &mut usize) -> Result<Option<i32>, String> {
    match read_u8(bytes, c)? {
        0 => Ok(None),
        1 => {
            let end = c.checked_add(4).ok_or("length overflow")?;
            let raw: [u8; 4] = bytes
                .get(*c..end)
                .ok_or("unexpected end of payload")?
                .try_into()
                .map_err(|_| "unexpected end of payload")?;
            *c = end;
            Ok(Some(i32::from_le_bytes(raw)))
        }
        other => Err(format!("invalid option tag {other}")),
    }
}

fn read_option_u16(bytes: &[u8], c: &mut usize) -> Result<Option<u16>, String> {
    match read_u8(bytes, c)? {
        0 => Ok(None),
        1 => {
            let end = c.checked_add(2).ok_or("length overflow")?;
            let raw: [u8; 2] = bytes
                .get(*c..end)
                .ok_or("unexpected end of payload")?
                .try_into()
                .map_err(|_| "unexpected end of payload")?;
            *c = end;
            Ok(Some(u16::from_le_bytes(raw)))
        }
        other => Err(format!("invalid option tag {other}")),
    }
}

fn read_resource_accounting_method(
    bytes: &[u8],
    c: &mut usize,
) -> Result<ResourceAccountingMethod, String> {
    match read_u8(bytes, c)? {
        0 => Ok(ResourceAccountingMethod::CgroupV2),
        1 => Ok(ResourceAccountingMethod::ProcfsRssSum),
        other => Err(format!("invalid resource accounting method {other}")),
    }
}

fn read_resource_accounting_coverage(
    bytes: &[u8],
    c: &mut usize,
) -> Result<ResourceAccountingCoverage, String> {
    match read_u8(bytes, c)? {
        0 => Ok(ResourceAccountingCoverage::Exact),
        1 => Ok(ResourceAccountingCoverage::BroaderThanTrace),
        2 => Ok(ResourceAccountingCoverage::Partial),
        other => Err(format!("invalid resource accounting coverage {other}")),
    }
}

fn read_resource_sample_kind(bytes: &[u8], c: &mut usize) -> Result<ResourceSampleKind, String> {
    match read_u8(bytes, c)? {
        0 => Ok(ResourceSampleKind::Periodic),
        1 => Ok(ResourceSampleKind::Final),
        other => Err(format!("invalid resource sample kind {other}")),
    }
}

fn read_option_memory_events(
    bytes: &[u8],
    c: &mut usize,
) -> Result<Option<MemoryEventCounters>, String> {
    match read_u8(bytes, c)? {
        0 => Ok(None),
        1 => Ok(Some(MemoryEventCounters {
            low: read_option_u64(bytes, c)?,
            high: read_option_u64(bytes, c)?,
            max: read_option_u64(bytes, c)?,
            oom: read_option_u64(bytes, c)?,
            oom_kill: read_option_u64(bytes, c)?,
            oom_group_kill: read_option_u64(bytes, c)?,
        })),
        other => Err(format!("invalid memory events option tag {other}")),
    }
}

fn read_option_body(bytes: &[u8], c: &mut usize) -> Result<Option<ApplicationBody>, String> {
    match read_u8(bytes, c)? {
        0 => Ok(None),
        1 => Ok(Some(ApplicationBody::Text(read_string(bytes, c)?))),
        2 => Ok(Some(ApplicationBody::Json(read_string(bytes, c)?))),
        3 => Ok(Some(ApplicationBody::Base64(read_string(bytes, c)?))),
        other => Err(format!("invalid application body tag {other}")),
    }
}

fn read_map(bytes: &[u8], c: &mut usize) -> Result<BTreeMap<String, String>, String> {
    let count = usize::try_from(read_varint(bytes, c)?).map_err(|_| "map length overflow")?;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let code = read_u8(bytes, c)?;
        let key = if code == UNKNOWN_KEY {
            read_string(bytes, c)?
        } else {
            MetadataKeyCodebook::key(code)
                .ok_or_else(|| format!("unknown metadata key code {code}"))?
                .to_string()
        };
        let value = read_string(bytes, c)?;
        map.insert(key, value);
    }
    Ok(map)
}

fn read_legacy_map(bytes: &[u8], c: &mut usize) -> Result<BTreeMap<String, String>, String> {
    let count = usize::try_from(read_varint(bytes, c)?).map_err(|_| "map length overflow")?;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let code = read_u8(bytes, c)?;
        let key = if code == UNKNOWN_KEY {
            read_legacy_string(bytes, c)?
        } else {
            MetadataKeyCodebook::key(code)
                .ok_or_else(|| format!("unknown metadata key code {code}"))?
                .to_string()
        };
        let value = read_legacy_string(bytes, c)?;
        map.insert(key, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Frozen output from the pre-ResourceV2 tag-6 encoder for:
    // Resource(process, pid:42, cpu=1250, rss=64, vm=None, {future_key=kept}).
    const LEGACY_RESOURCE_FIXTURE: &[u8] = &[
        6, 7, b'p', b'r', b'o', b'c', b'e', b's', b's', 6, b'p', b'i', b'd', b':', b'4', b'2', 1,
        0xe2, 0x04, 0, 0, 0, 0, 0, 0, 1, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xff, 10, b'f', b'u',
        b't', b'u', b'r', b'e', b'_', b'k', b'e', b'y', 4, b'k', b'e', b'p', b't',
    ];

    #[test]
    fn legacy_resource_fixture_decodes_with_explicit_compatibility_semantics() {
        let decoded = ManualCodec.decode(LEGACY_RESOURCE_FIXTURE).unwrap();
        let EventPayload::Resource(payload) = decoded else {
            panic!("expected resource payload");
        };
        assert_eq!(payload.scope, "process");
        assert_eq!(payload.subject, "pid:42");
        assert_eq!(
            payload.accounting_method,
            ResourceAccountingMethod::ProcfsRssSum
        );
        assert_eq!(
            payload.accounting_coverage,
            ResourceAccountingCoverage::Partial
        );
        assert_eq!(payload.sample_kind, ResourceSampleKind::Periodic);
        assert_eq!(payload.cpu_percent_millis, Some(1_250));
        assert_eq!(payload.rss_kb, Some(64));
        assert_eq!(payload.virtual_memory_kb, None);
        assert_eq!(payload.memory_current_bytes, None);
        assert_eq!(
            payload.metadata.get("future_key").map(String::as_str),
            Some("kept")
        );
    }

    #[test]
    fn optimized_legacy_resource_fixture_remains_readable() {
        let mut fixture = vec![RESOURCE_LEGACY_TAG];
        write_string(&mut fixture, "process");
        write_string(&mut fixture, "pid:42");
        write_option_u64(&mut fixture, Some(1_250));
        write_option_u64(&mut fixture, Some(64));
        write_option_u64(&mut fixture, None);
        write_map(
            &mut fixture,
            &BTreeMap::from([("future_key".to_string(), "kept".to_string())]),
        );

        let decoded = ManualCodec.decode(&fixture).unwrap();
        let EventPayload::Resource(payload) = decoded else {
            panic!("expected resource payload");
        };
        assert_eq!(payload.scope, "process");
        assert_eq!(payload.subject, "pid:42");
        assert_eq!(payload.rss_kb, Some(64));
        assert_eq!(
            payload.metadata.get("future_key").map(String::as_str),
            Some("kept")
        );
    }

    #[test]
    fn resource_v2_round_trips_all_fields_and_unknown_metadata() {
        let payload = EventPayload::Resource(ResourcePayload {
            scope: "trace".to_string(),
            subject: "trace:7".to_string(),
            accounting_method: ResourceAccountingMethod::CgroupV2,
            accounting_coverage: ResourceAccountingCoverage::Exact,
            sample_kind: ResourceSampleKind::Final,
            cpu_percent_millis: Some(7),
            rss_kb: Some(8),
            virtual_memory_kb: Some(9),
            memory_current_bytes: Some(10),
            memory_peak_bytes: Some(11),
            memory_anon_bytes: Some(12),
            memory_file_bytes: Some(13),
            memory_swap_current_bytes: Some(14),
            memory_events: Some(memory_events(20)),
            memory_events_local: Some(memory_events(30)),
            cpu_usage_usec: Some(40),
            cpu_user_usec: Some(41),
            cpu_system_usec: Some(42),
            cpu_nr_throttled: Some(43),
            cpu_throttled_usec: Some(44),
            io_read_bytes: Some(50),
            io_write_bytes: Some(51),
            pids_current: Some(60),
            pids_peak: Some(61),
            process_rss_sum_kb: Some(70),
            metadata: BTreeMap::from([("future.resource.key".to_string(), "kept".to_string())]),
        });

        let encoded = ManualCodec.encode(&payload).unwrap();
        assert_eq!(encoded.first(), Some(&RESOURCE_V2_TAG));
        assert_eq!(ManualCodec.decode(&encoded).unwrap(), payload);
    }

    #[test]
    fn resource_v2_round_trips_absent_optional_fields() {
        let payload = EventPayload::Resource(ResourcePayload {
            scope: "process_tree".to_string(),
            subject: "pid:99".to_string(),
            ..ResourcePayload::default()
        });
        let encoded = ManualCodec.encode(&payload).unwrap();
        assert_eq!(ManualCodec.decode(&encoded).unwrap(), payload);
    }

    #[test]
    fn resource_v2_rejects_truncation_and_trailing_bytes() {
        let payload = EventPayload::Resource(ResourcePayload {
            scope: "trace".to_string(),
            subject: "trace:9".to_string(),
            ..ResourcePayload::default()
        });
        let encoded = ManualCodec.encode(&payload).unwrap();
        assert!(ManualCodec.decode(&encoded[..encoded.len() - 1]).is_err());

        let mut with_trailing = encoded;
        with_trailing.push(0);
        assert_eq!(
            ManualCodec.decode(&with_trailing).unwrap_err(),
            "trailing bytes after event payload"
        );
    }

    #[test]
    fn historical_resource_json_uses_scoped_defaults() {
        let payload: ResourcePayload = serde_json::from_str(
            r#"{"scope":"process","subject":"pid:1","cpu_percent_millis":null,"rss_kb":12,"virtual_memory_kb":24,"metadata":{}}"#,
        )
        .unwrap();
        assert_eq!(
            payload.accounting_method,
            ResourceAccountingMethod::ProcfsRssSum
        );
        assert_eq!(
            payload.accounting_coverage,
            ResourceAccountingCoverage::Partial
        );
        assert_eq!(payload.sample_kind, ResourceSampleKind::Periodic);
    }

    fn memory_events(base: u64) -> MemoryEventCounters {
        MemoryEventCounters {
            low: Some(base),
            high: Some(base + 1),
            max: Some(base + 2),
            oom: Some(base + 3),
            oom_kill: Some(base + 4),
            oom_group_kill: Some(base + 5),
        }
    }
}

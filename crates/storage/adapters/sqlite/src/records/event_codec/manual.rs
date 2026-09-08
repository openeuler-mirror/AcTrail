//! Hand-written compact binary codec for event payloads.
//!
//! Replaces bincode with a fixed, minimal wire format: a `u8` variant tag
//! (bincode uses `u32`), LEB128 length prefixes, and a static metadata-key
//! dictionary so the common short keys (`operation`, `direction`, `fd`, ...)
//! store as one byte instead of repeating the key name per event.

use std::collections::BTreeMap;

use model_core::event::{
    ApplicationBody, ApplicationPayload, ControlPayload, EnforcementPayload, EventPayload,
    FilePayload, IpcPayload, LabelPayload, LossPayload, NetPayload, ProcessPayload,
    ResourcePayload, StdioPayload,
};

use super::key_codes::MetadataKeyCodebook;
use super::value_codes::{value_code, value_for_code};
use model_core::process::ProcessIdentity;

use super::EventPayloadCodec;

const UNKNOWN_KEY: u8 = 0xFF;

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
                out.push(6);
                encode_resource(&mut out, p);
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
            6 => EventPayload::Resource(decode_resource(bytes, &mut cursor)?),
            7 => EventPayload::Control(decode_control(bytes, &mut cursor)?),
            8 => EventPayload::Loss(decode_loss(bytes, &mut cursor)?),
            9 => EventPayload::Label(decode_label(bytes, &mut cursor)?),
            10 => EventPayload::Enforcement(decode_enforcement(bytes, &mut cursor)?),
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

fn encode_resource(out: &mut Vec<u8>, p: &ResourcePayload) {
    write_string(out, &p.scope);
    write_string(out, &p.subject);
    write_option_u64(out, p.cpu_percent_millis);
    write_option_u64(out, p.rss_kb);
    write_option_u64(out, p.virtual_memory_kb);
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

fn decode_resource(bytes: &[u8], c: &mut usize) -> Result<ResourcePayload, String> {
    Ok(ResourcePayload {
        scope: read_string(bytes, c)?,
        subject: read_string(bytes, c)?,
        cpu_percent_millis: read_option_u64(bytes, c)?,
        rss_kb: read_option_u64(bytes, c)?,
        virtual_memory_kb: read_option_u64(bytes, c)?,
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

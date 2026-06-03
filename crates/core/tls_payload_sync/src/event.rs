//! Versioned line codec for sync TLS runtime events.

use std::str::FromStr;

use tls_payload_core::PayloadDirection;

use crate::{SyncError, SyncResult};

const EVENT_VERSION: &str = "v1";
const FIELD_SEPARATOR: char = '\t';
const PAYLOAD_OPCODE: &str = "payload";
const DECISION_OPCODE: &str = "decision";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadEvent {
    pub trace_id: u64,
    pub pid: u32,
    pub direction: PayloadDirection,
    pub provider: String,
    pub symbol: String,
    pub stream_key: u64,
    pub sequence: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionEvent {
    pub trace_id: u64,
    pub pid: u32,
    pub direction: PayloadDirection,
    pub provider: String,
    pub symbol: String,
    pub stream_key: u64,
    pub sequence: u64,
    pub action: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncEvent {
    Payload(PayloadEvent),
    Decision(DecisionEvent),
}

pub fn encode_event_line(event: &SyncEvent) -> Vec<u8> {
    let fields = match event {
        SyncEvent::Payload(event) => vec![
            EVENT_VERSION.to_string(),
            PAYLOAD_OPCODE.to_string(),
            event.trace_id.to_string(),
            event.pid.to_string(),
            event.direction.as_str().to_string(),
            event.provider.clone(),
            event.symbol.clone(),
            event.stream_key.to_string(),
            event.sequence.to_string(),
            encode_hex(&event.bytes),
        ],
        SyncEvent::Decision(event) => vec![
            EVENT_VERSION.to_string(),
            DECISION_OPCODE.to_string(),
            event.trace_id.to_string(),
            event.pid.to_string(),
            event.direction.as_str().to_string(),
            event.provider.clone(),
            event.symbol.clone(),
            event.stream_key.to_string(),
            event.sequence.to_string(),
            event.action.clone(),
            encode_hex(event.reason.as_bytes()),
        ],
    };
    let mut line = fields.join(&FIELD_SEPARATOR.to_string()).into_bytes();
    line.push(b'\n');
    line
}

pub fn decode_event_line(line: &[u8]) -> SyncResult<SyncEvent> {
    let line = std::str::from_utf8(line)
        .map_err(|error| SyncError::new(format!("sync event utf8: {error}")))?
        .trim_end_matches('\n');
    let fields = line.split(FIELD_SEPARATOR).collect::<Vec<_>>();
    if fields.first().copied() != Some(EVENT_VERSION) {
        return Err(SyncError::new("unsupported sync event version"));
    }
    match fields.get(1).copied() {
        Some(PAYLOAD_OPCODE) => decode_payload(&fields),
        Some(DECISION_OPCODE) => decode_decision(&fields),
        _ => Err(SyncError::new("unknown sync event opcode")),
    }
}

fn decode_payload(fields: &[&str]) -> SyncResult<SyncEvent> {
    require_len(fields, 10, PAYLOAD_OPCODE)?;
    Ok(SyncEvent::Payload(PayloadEvent {
        trace_id: parse(fields[2], "trace_id")?,
        pid: parse(fields[3], "pid")?,
        direction: PayloadDirection::from_str(fields[4])
            .map_err(|error| SyncError::new(error.to_string()))?,
        provider: fields[5].to_string(),
        symbol: fields[6].to_string(),
        stream_key: parse(fields[7], "stream_key")?,
        sequence: parse(fields[8], "sequence")?,
        bytes: decode_hex(fields[9])?,
    }))
}

fn decode_decision(fields: &[&str]) -> SyncResult<SyncEvent> {
    require_len(fields, 11, DECISION_OPCODE)?;
    let reason = String::from_utf8(decode_hex(fields[10])?)
        .map_err(|error| SyncError::new(format!("decision reason utf8: {error}")))?;
    Ok(SyncEvent::Decision(DecisionEvent {
        trace_id: parse(fields[2], "trace_id")?,
        pid: parse(fields[3], "pid")?,
        direction: PayloadDirection::from_str(fields[4])
            .map_err(|error| SyncError::new(error.to_string()))?,
        provider: fields[5].to_string(),
        symbol: fields[6].to_string(),
        stream_key: parse(fields[7], "stream_key")?,
        sequence: parse(fields[8], "sequence")?,
        action: fields[9].to_string(),
        reason,
    }))
}

fn require_len(fields: &[&str], expected: usize, opcode: &str) -> SyncResult<()> {
    if fields.len() == expected {
        Ok(())
    } else {
        Err(SyncError::new(format!(
            "invalid {opcode} sync event field count {}",
            fields.len()
        )))
    }
}

fn parse<T>(value: &str, name: &str) -> SyncResult<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse::<T>()
        .map_err(|error| SyncError::new(format!("parse {name}: {error}")))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

fn decode_hex(value: &str) -> SyncResult<Vec<u8>> {
    if value.len() % 2 != 0 {
        return Err(SyncError::new("hex value has odd length"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk)
            .map_err(|error| SyncError::new(format!("hex utf8: {error}")))?;
        bytes.push(
            u8::from_str_radix(text, 16)
                .map_err(|error| SyncError::new(format!("hex {text}: {error}")))?,
        );
    }
    Ok(bytes)
}

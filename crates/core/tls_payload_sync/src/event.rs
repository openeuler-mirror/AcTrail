//! Versioned line codec for sync TLS runtime events.

use std::io::Write;
use std::str::FromStr;

use tls_payload_core::PayloadDirection;

use crate::{SyncError, SyncResult};

const EVENT_VERSION: &str = "v1";
const FIELD_SEPARATOR: char = '\t';
const PAYLOAD_OPCODE: &str = "payload";
const DECISION_OPCODE: &str = "decision";
const SUMMARY_OPCODE: &str = "summary";
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

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
pub struct SummaryEvent {
    pub trace_id: u64,
    pub pid: u32,
    pub direction: PayloadDirection,
    pub provider: String,
    pub symbol: String,
    pub stream_key: u64,
    pub sequence: u64,
    pub observed_size: u64,
    pub emitted_size: u64,
    pub reason: String,
    pub protocol_hint: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncEvent {
    Payload(PayloadEvent),
    Decision(DecisionEvent),
    Summary(SummaryEvent),
}

pub fn encode_event_line(event: &SyncEvent) -> Vec<u8> {
    let mut line = Vec::new();
    write_event_line(&mut line, event).expect("writing to Vec should not fail");
    line
}

pub fn write_event_line(writer: &mut impl Write, event: &SyncEvent) -> SyncResult<()> {
    match event {
        SyncEvent::Payload(event) => {
            write_fields(
                writer,
                &[
                    EVENT_VERSION,
                    PAYLOAD_OPCODE,
                    &event.trace_id.to_string(),
                    &event.pid.to_string(),
                    event.direction.as_str(),
                    &event.provider,
                    &event.symbol,
                    &event.stream_key.to_string(),
                    &event.sequence.to_string(),
                ],
            )?;
            write_hex(writer, &event.bytes)?;
        }
        SyncEvent::Decision(event) => {
            write_fields(
                writer,
                &[
                    EVENT_VERSION,
                    DECISION_OPCODE,
                    &event.trace_id.to_string(),
                    &event.pid.to_string(),
                    event.direction.as_str(),
                    &event.provider,
                    &event.symbol,
                    &event.stream_key.to_string(),
                    &event.sequence.to_string(),
                    &event.action,
                ],
            )?;
            write_hex(writer, event.reason.as_bytes())?;
        }
        SyncEvent::Summary(event) => {
            write_fields(
                writer,
                &[
                    EVENT_VERSION,
                    SUMMARY_OPCODE,
                    &event.trace_id.to_string(),
                    &event.pid.to_string(),
                    event.direction.as_str(),
                    &event.provider,
                    &event.symbol,
                    &event.stream_key.to_string(),
                    &event.sequence.to_string(),
                    &event.observed_size.to_string(),
                    &event.emitted_size.to_string(),
                    &event.reason,
                    &event.protocol_hint,
                ],
            )?;
            write_hex(writer, &event.bytes)?;
        }
    }
    writer.write_all(b"\n").map_err(sync_io_error)
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
        Some(SUMMARY_OPCODE) => decode_summary(&fields),
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

fn decode_summary(fields: &[&str]) -> SyncResult<SyncEvent> {
    require_len(fields, 14, SUMMARY_OPCODE)?;
    Ok(SyncEvent::Summary(SummaryEvent {
        trace_id: parse(fields[2], "trace_id")?,
        pid: parse(fields[3], "pid")?,
        direction: PayloadDirection::from_str(fields[4])
            .map_err(|error| SyncError::new(error.to_string()))?,
        provider: fields[5].to_string(),
        symbol: fields[6].to_string(),
        stream_key: parse(fields[7], "stream_key")?,
        sequence: parse(fields[8], "sequence")?,
        observed_size: parse(fields[9], "observed_size")?,
        emitted_size: parse(fields[10], "emitted_size")?,
        reason: fields[11].to_string(),
        protocol_hint: fields[12].to_string(),
        bytes: decode_hex(fields[13])?,
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

fn write_fields(writer: &mut impl Write, fields: &[&str]) -> SyncResult<()> {
    for (index, field) in fields.iter().enumerate() {
        if index != 0 {
            write_separator(writer)?;
        }
        writer.write_all(field.as_bytes()).map_err(sync_io_error)?;
    }
    write_separator(writer)
}

fn write_separator(writer: &mut impl Write) -> SyncResult<()> {
    let separator = [FIELD_SEPARATOR as u8];
    writer.write_all(&separator).map_err(sync_io_error)
}

fn write_hex(writer: &mut impl Write, bytes: &[u8]) -> SyncResult<()> {
    for byte in bytes {
        let encoded = [
            HEX_DIGITS[(byte >> 4) as usize],
            HEX_DIGITS[(byte & 0x0f) as usize],
        ];
        writer.write_all(&encoded).map_err(sync_io_error)?;
    }
    Ok(())
}

fn sync_io_error(error: std::io::Error) -> SyncError {
    SyncError::new(error.to_string())
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

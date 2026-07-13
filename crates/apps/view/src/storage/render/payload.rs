//! Payload rendering.

use model_core::payload::{PayloadDirection, PayloadSegment};

use crate::command::PayloadFormat;
use crate::table::Table;

pub(super) fn render_payloads(segments: Vec<PayloadSegment>) -> String {
    let mut table = Table::new(&[
        "SEGMENT",
        "PROCESS",
        "DIRECTION",
        "STATE",
        "SIZE",
        "FLAGS",
        "OPERATION",
        "SOURCE",
        "LIBRARY",
        "SYMBOL",
    ]);
    for segment in segments {
        table.push(vec![
            segment.segment_id.to_string(),
            segment.process.get().to_string(),
            payload_direction(segment.direction).to_string(),
            format!("{:?}", segment.content_state),
            format!("{}/{}", segment.captured_size, segment.original_size),
            format!("{:?}/{:?}", segment.truncation, segment.redaction),
            format!(
                "{}@{} {}/{} {}",
                segment.operation_id,
                segment.operation_offset,
                segment.operation_captured_size,
                segment.operation_original_size,
                segment.operation_completion_state.as_str()
            ),
            format!("{:?}", segment.source_boundary),
            segment.library,
            segment.symbol,
        ]);
    }
    render_table(table, "no payload segments")
}

pub(super) fn render_payload(segment: PayloadSegment, format: PayloadFormat) -> String {
    match format {
        PayloadFormat::Text => String::from_utf8_lossy(&segment.bytes).into_owned(),
        PayloadFormat::Hex => render_hex(&segment.bytes),
    }
}

fn payload_direction(direction: PayloadDirection) -> &'static str {
    match direction {
        PayloadDirection::Outbound => "outbound",
        PayloadDirection::Inbound => "inbound",
    }
}

fn render_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    output
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '?',
    }
}

fn render_table(table: Table, empty_message: &str) -> String {
    if table.is_empty() {
        empty_message.to_string()
    } else {
        table.render()
    }
}

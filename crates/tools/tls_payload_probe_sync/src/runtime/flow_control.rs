//! Low-cost TLS plaintext flow classification.

use std::collections::BTreeMap;

use tls_payload_core::PayloadDirection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FlowControlConfig {
    pub(in crate::runtime) enabled: bool,
    pub(in crate::runtime) sniff_bytes: usize,
    pub(in crate::runtime) max_header_bytes: usize,
    pub(in crate::runtime) large_transfer_bytes: u64,
    pub(in crate::runtime) unknown_stream_bytes: u64,
    pub(in crate::runtime) h2_data_probe_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FlowDecision {
    EmitPayload,
    EmitSummary(FlowSummary),
    DropBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FlowSummary {
    pub(in crate::runtime) observed_size: u64,
    pub(in crate::runtime) reason: &'static str,
    pub(in crate::runtime) protocol_hint: &'static str,
    pub(in crate::runtime) bytes: Vec<u8>,
}

#[derive(Debug, Default)]
pub(in crate::runtime) struct FlowController {
    streams: BTreeMap<FlowKey, FlowState>,
}

impl FlowController {
    pub(in crate::runtime) fn observe(
        &mut self,
        config: FlowControlConfig,
        direction: PayloadDirection,
        stream_key: usize,
        payload: &[u8],
    ) -> FlowDecision {
        if !config.enabled || payload.is_empty() {
            return FlowDecision::EmitPayload;
        }
        let key = FlowKey {
            stream_key,
            direction: FlowDirection::from(direction),
        };
        let state = self.streams.entry(key).or_default();
        match state {
            FlowState::SummaryOnly { observed } => {
                *observed = observed.saturating_add(payload.len() as u64);
                return FlowDecision::DropBody;
            }
            FlowState::Active { observed, prefix } => {
                *observed = observed.saturating_add(payload.len() as u64);
                append_prefix(prefix, payload, config.sniff_bytes);
                if let Some(summary) = classify_immediate(config, direction, *observed, prefix) {
                    *state = FlowState::SummaryOnly {
                        observed: summary.observed_size,
                    };
                    return FlowDecision::EmitSummary(summary);
                }
                if *observed > config.unknown_stream_bytes && unknown_prefix(prefix) {
                    let summary = FlowSummary {
                        observed_size: *observed,
                        reason: "unknown_stream_threshold",
                        protocol_hint: "unknown",
                        bytes: Vec::new(),
                    };
                    *state = FlowState::SummaryOnly {
                        observed: summary.observed_size,
                    };
                    return FlowDecision::EmitSummary(summary);
                }
            }
        }
        FlowDecision::EmitPayload
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FlowKey {
    stream_key: usize,
    direction: FlowDirection,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum FlowDirection {
    Outbound,
    Inbound,
}

impl From<PayloadDirection> for FlowDirection {
    fn from(value: PayloadDirection) -> Self {
        match value {
            PayloadDirection::Outbound => Self::Outbound,
            PayloadDirection::Inbound => Self::Inbound,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FlowState {
    Active { observed: u64, prefix: Vec<u8> },
    SummaryOnly { observed: u64 },
}

impl Default for FlowState {
    fn default() -> Self {
        Self::Active {
            observed: 0,
            prefix: Vec::new(),
        }
    }
}

fn append_prefix(prefix: &mut Vec<u8>, payload: &[u8], limit: usize) {
    if prefix.len() >= limit {
        return;
    }
    let remaining = limit - prefix.len();
    prefix.extend_from_slice(&payload[..payload.len().min(remaining)]);
}

fn classify_immediate(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    payload: &[u8],
) -> Option<FlowSummary> {
    classify_http1(config, direction, observed, payload)
        .or_else(|| classify_http2(config, direction, observed, payload))
        .or_else(|| classify_binary_prefix(config, observed, payload))
}

fn classify_http1(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    payload: &[u8],
) -> Option<FlowSummary> {
    let header_end = http1_header_end(payload)?;
    if header_end > config.max_header_bytes {
        return Some(FlowSummary {
            observed_size: observed,
            reason: "http1_header_too_large",
            protocol_hint: "http/1.x",
            bytes: Vec::new(),
        });
    }
    let header = &payload[..header_end];
    if !looks_like_http1_header(header) {
        return None;
    }
    let body = &payload[header_end..];
    let content_length = http1_header_value_u64(header, b"content-length");
    let binary_content_type = http1_header_value_contains_any(
        header,
        b"content-type",
        &[b"octet-stream", b"application/zip", b"application/gzip"],
    );
    let attachment =
        http1_header_value_contains_any(header, b"content-disposition", &[b"attachment"]);
    let body_binary = !body.is_empty() && body_looks_binary(body);
    let large = content_length
        .map(|value| value > config.large_transfer_bytes)
        .unwrap_or(observed > config.large_transfer_bytes);
    let large_binary_header = large && (binary_content_type || attachment);
    let should_summarize = match direction {
        PayloadDirection::Inbound => {
            large_binary_header
                || body_binary
                || (large && !body.is_empty() && !body_looks_text_api(body))
        }
        PayloadDirection::Outbound => large && (binary_content_type || attachment || body_binary),
    };
    if !should_summarize {
        return None;
    }
    Some(FlowSummary {
        observed_size: content_length.unwrap_or(observed),
        reason: if body_binary {
            "binary_body"
        } else {
            "large_non_text_transfer"
        },
        protocol_hint: "http/1.x",
        bytes: header.to_vec(),
    })
}

fn classify_http2(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    payload: &[u8],
) -> Option<FlowSummary> {
    let mut cursor = if payload.starts_with(HTTP2_CONNECTION_PREFACE) {
        HTTP2_CONNECTION_PREFACE.len()
    } else {
        0
    };
    let mut saw_frame = false;
    let mut data_bytes = 0_u64;
    let mut data_prefix = Vec::new();
    while cursor + HTTP2_FRAME_HEADER_BYTES <= payload.len() {
        let frame = decode_h2_frame(&payload[cursor..])?;
        saw_frame = true;
        if frame.frame_type == HTTP2_DATA_FRAME_TYPE {
            data_bytes = data_bytes.saturating_add(frame.payload.len() as u64);
            append_prefix(&mut data_prefix, frame.payload, config.sniff_bytes);
        }
        cursor += frame.encoded_len;
    }
    if !saw_frame || data_bytes == 0 {
        return None;
    }
    let data_over_probe =
        data_bytes > config.h2_data_probe_bytes || observed > config.h2_data_probe_bytes;
    let binary = body_looks_binary(&data_prefix);
    if matches!(direction, PayloadDirection::Inbound)
        && (binary || data_over_probe && !body_looks_text_api(&data_prefix))
    {
        return Some(FlowSummary {
            observed_size: observed,
            reason: if binary {
                "h2_binary_data"
            } else {
                "h2_data_probe_exceeded"
            },
            protocol_hint: "h2",
            bytes: Vec::new(),
        });
    }
    None
}

fn classify_binary_prefix(
    config: FlowControlConfig,
    observed: u64,
    payload: &[u8],
) -> Option<FlowSummary> {
    if observed < config.unknown_stream_bytes || !body_looks_binary(payload) {
        return None;
    }
    Some(FlowSummary {
        observed_size: observed,
        reason: "binary_unknown_stream",
        protocol_hint: "unknown",
        bytes: Vec::new(),
    })
}

fn unknown_prefix(prefix: &[u8]) -> bool {
    !looks_like_http1_header(prefix) && !prefix.starts_with(HTTP2_CONNECTION_PREFACE)
}

fn http1_header_end(bytes: &[u8]) -> Option<usize> {
    find_bytes(bytes, b"\r\n\r\n")
        .map(|index| index + b"\r\n\r\n".len())
        .or_else(|| find_bytes(bytes, b"\n\n").map(|index| index + b"\n\n".len()))
}

fn looks_like_http1_header(bytes: &[u8]) -> bool {
    let Some(line_end) = find_byte(bytes, b'\n') else {
        return false;
    };
    let first = trim_ascii(&bytes[..line_end]);
    if first.starts_with(b"HTTP/") {
        return true;
    }
    let Some((method, rest)) = split_once_space(first) else {
        return false;
    };
    let Some((_, version)) = split_once_space(trim_ascii(rest)) else {
        return false;
    };
    method
        .iter()
        .all(|byte| byte.is_ascii_uppercase() || *byte == b'-')
        && version.starts_with(b"HTTP/")
}

fn http1_header_value_u64(header: &[u8], name: &[u8]) -> Option<u64> {
    http1_header_value(header, name).and_then(|value| {
        std::str::from_utf8(trim_ascii(value))
            .ok()?
            .parse::<u64>()
            .ok()
    })
}

fn http1_header_value_contains_any(header: &[u8], name: &[u8], needles: &[&[u8]]) -> bool {
    let Some(value) = http1_header_value(header, name) else {
        return false;
    };
    let value = ascii_lowercase(value);
    needles
        .iter()
        .any(|needle| find_bytes(&value, needle).is_some())
}

fn http1_header_value<'a>(header: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    for line in header.split(|byte| *byte == b'\n') {
        let line = trim_ascii(line);
        let Some(colon) = find_byte(line, b':') else {
            continue;
        };
        let key = trim_ascii(&line[..colon]);
        if eq_ignore_ascii_case(key, name) {
            return Some(trim_ascii(&line[colon + 1..]));
        }
    }
    None
}

fn body_looks_text_api(bytes: &[u8]) -> bool {
    let bytes = trim_ascii(bytes);
    if bytes.is_empty() {
        return true;
    }
    bytes.starts_with(b"{")
        || bytes.starts_with(b"[")
        || bytes.starts_with(b"data:")
        || bytes.starts_with(b"event:")
        || text_ratio(bytes) > 900
}

fn body_looks_binary(bytes: &[u8]) -> bool {
    let bytes = trim_ascii(bytes);
    if bytes.is_empty() {
        return false;
    }
    if bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"\x1f\x8b")
        || bytes.starts_with(b"\x28\xb5\x2f\xfd")
        || bytes.starts_with(b"\0asm")
        || bytes.starts_with(b"%PDF")
    {
        return true;
    }
    text_ratio(bytes) < 700
}

fn text_ratio(bytes: &[u8]) -> u16 {
    let sample = &bytes[..bytes.len().min(4096)];
    if sample.is_empty() {
        return 1000;
    }
    let text = sample
        .iter()
        .filter(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace())
        .count();
    ((text * 1000) / sample.len()) as u16
}

const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const HTTP2_FRAME_HEADER_BYTES: usize = 9;
const HTTP2_DATA_FRAME_TYPE: u8 = 0;

struct H2Frame<'a> {
    frame_type: u8,
    encoded_len: usize,
    payload: &'a [u8],
}

fn decode_h2_frame(bytes: &[u8]) -> Option<H2Frame<'_>> {
    if bytes.len() < HTTP2_FRAME_HEADER_BYTES {
        return None;
    }
    let len = ((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize;
    let end = HTTP2_FRAME_HEADER_BYTES.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }
    let stream_id = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) & 0x7fff_ffff;
    let frame_type = bytes[3];
    if stream_id == 0 && matches!(frame_type, HTTP2_DATA_FRAME_TYPE) {
        return None;
    }
    Some(H2Frame {
        frame_type,
        encoded_len: end,
        payload: &bytes[HTTP2_FRAME_HEADER_BYTES..end],
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_byte(bytes: &[u8], needle: u8) -> Option<usize> {
    bytes.iter().position(|byte| *byte == needle)
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn split_once_space(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|byte| byte.is_ascii_whitespace())?;
    Some((&bytes[..index], &bytes[index + 1..]))
}

fn eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn ascii_lowercase(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

use tls_payload_core::PayloadDirection;

use super::text::{
    ascii_lowercase, body_looks_binary, body_looks_text_api, eq_ignore_ascii_case, find_byte,
    find_bytes, split_once_space, trim_ascii,
};
use super::types::{FlowControlConfig, FlowSummary};

pub(super) struct Http1Inspection {
    pub(super) message_size: Option<u64>,
    pub(super) summary: Option<FlowSummary>,
}

pub(super) fn inspect(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    payload: &[u8],
) -> Option<Http1Inspection> {
    let header_end = header_end(payload)?;
    if header_end > config.max_header_bytes {
        return Some(Http1Inspection {
            message_size: None,
            summary: Some(FlowSummary {
                observed_size: observed,
                reason: "http1_header_too_large",
                protocol_hint: "http/1.x",
                bytes: Vec::new(),
            }),
        });
    }
    let header = &payload[..header_end];
    if !looks_like_header(header) {
        return None;
    }
    let content_length = header_value_u64(header, b"content-length");
    let message_size = content_length.and_then(|length| (header_end as u64).checked_add(length));
    let body = body_prefix(payload, header_end, message_size);
    let binary_content_type = header_value_contains_any(
        header,
        b"content-type",
        &[b"octet-stream", b"application/zip", b"application/gzip"],
    );
    let attachment = header_value_contains_any(header, b"content-disposition", &[b"attachment"]);
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
    let summary = should_summarize.then(|| FlowSummary {
        observed_size: content_length.unwrap_or(observed),
        reason: if body_binary {
            "binary_body"
        } else {
            "large_non_text_transfer"
        },
        protocol_hint: "http/1.x",
        bytes: header.to_vec(),
    });
    Some(Http1Inspection {
        message_size,
        summary,
    })
}

pub(super) fn looks_like_header(bytes: &[u8]) -> bool {
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

fn body_prefix(payload: &[u8], header_end: usize, message_size: Option<u64>) -> &[u8] {
    let body_end = message_size
        .and_then(|size| usize::try_from(size).ok())
        .map(|size| size.min(payload.len()))
        .unwrap_or(payload.len());
    if body_end <= header_end {
        return &[];
    }
    &payload[header_end..body_end]
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    find_bytes(bytes, b"\r\n\r\n")
        .map(|index| index + b"\r\n\r\n".len())
        .or_else(|| find_bytes(bytes, b"\n\n").map(|index| index + b"\n\n".len()))
}

fn header_value_u64(header: &[u8], name: &[u8]) -> Option<u64> {
    header_value(header, name).and_then(|value| {
        std::str::from_utf8(trim_ascii(value))
            .ok()?
            .parse::<u64>()
            .ok()
    })
}

fn header_value_contains_any(header: &[u8], name: &[u8], needles: &[&[u8]]) -> bool {
    let Some(value) = header_value(header, name) else {
        return false;
    };
    let value = ascii_lowercase(value);
    needles
        .iter()
        .any(|needle| find_bytes(&value, needle).is_some())
}

fn header_value<'a>(header: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
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

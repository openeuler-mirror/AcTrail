use tls_payload_core::PayloadDirection;

use super::text::{body_looks_binary, body_looks_text_api};
use super::types::{FlowControlConfig, FlowSummary};

const CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const FRAME_HEADER_BYTES: usize = 9;
const DATA_FRAME_TYPE: u8 = 0;

pub(super) fn classify(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    payload: &[u8],
) -> Option<FlowSummary> {
    let mut cursor = if payload.starts_with(CONNECTION_PREFACE) {
        CONNECTION_PREFACE.len()
    } else {
        0
    };
    let mut saw_frame = false;
    let mut data_bytes = 0_u64;
    let mut data_prefix = Vec::new();
    while cursor + FRAME_HEADER_BYTES <= payload.len() {
        let frame = decode_frame(&payload[cursor..])?;
        saw_frame = true;
        if frame.frame_type == DATA_FRAME_TYPE {
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

pub(super) fn starts_with_preface(bytes: &[u8]) -> bool {
    bytes.starts_with(CONNECTION_PREFACE)
}

fn append_prefix(prefix: &mut Vec<u8>, payload: &[u8], limit: usize) {
    if prefix.len() >= limit {
        return;
    }
    let remaining = limit - prefix.len();
    prefix.extend_from_slice(&payload[..payload.len().min(remaining)]);
}

struct H2Frame<'a> {
    frame_type: u8,
    encoded_len: usize,
    payload: &'a [u8],
}

fn decode_frame(bytes: &[u8]) -> Option<H2Frame<'_>> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return None;
    }
    let len = ((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize;
    let end = FRAME_HEADER_BYTES.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }
    let stream_id = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) & 0x7fff_ffff;
    let frame_type = bytes[3];
    if stream_id == 0 && matches!(frame_type, DATA_FRAME_TYPE) {
        return None;
    }
    Some(H2Frame {
        frame_type,
        encoded_len: end,
        payload: &bytes[FRAME_HEADER_BYTES..end],
    })
}

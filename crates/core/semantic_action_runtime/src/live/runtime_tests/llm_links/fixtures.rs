use model_core::process::ProcessIdentity;
use model_core::{
    event::{DomainEvent, EventPayload},
    ids::EventId,
    payload::{PayloadDirection, PayloadSegment, PayloadSegmentId},
};

use super::super::test_support::*;

const HTTP2_DATA_FRAME_TYPE: u8 = 0x0;
const HTTP2_HEADERS_FRAME_TYPE: u8 = 0x1;
const HTTP2_FRAME_HEADER_BYTES: usize = 9;
const HTTP2_LENGTH_BYTE_MASK: usize = 0xff;
const HTTP2_MAX_FRAME_LENGTH_FIELD: usize = 0x00ff_ffff;
const HTTP2_RESERVED_STREAM_ID_MASK: u32 = 0x7fff_ffff;

pub(super) fn http_response_event_with(
    event_id: EventId,
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    sequence: u64,
    status_code: &str,
    reason: &str,
) -> DomainEvent {
    let mut event = http_response_event(event_id, process);
    let EventPayload::Application(payload) = &mut event.payload else {
        unreachable!("http_response_event returns an application event");
    };
    payload.summary = format!("{status_code} {reason}");
    payload.metadata.insert(
        "payload_segment_id".to_string(),
        segment_id.get().to_string(),
    );
    payload
        .metadata
        .insert("payload_sequence".to_string(), sequence.to_string());
    payload
        .metadata
        .insert("status_code".to_string(), status_code.to_string());
    payload
        .metadata
        .insert("reason".to_string(), reason.to_string());
    event
}

pub(super) fn split_request_segment(process: ProcessIdentity) -> (PayloadSegment, PayloadSegment) {
    let mut head = llm_payload_segment(process);
    let split_at = head.bytes.len() / 2;
    let tail_bytes = head.bytes.split_off(split_at);
    let head_bytes = std::mem::take(&mut head.bytes);
    head.segment_id = response_segment_id(18);
    head.operation_id = response_operation_id(18);
    head.sequence = response_sequence(18);
    set_segment_bytes(&mut head, head_bytes, 0);

    let mut tail = head.clone();
    tail.segment_id = response_segment_id(21);
    tail.operation_id = response_operation_id(21);
    tail.sequence = response_sequence(21);
    set_segment_bytes(&mut tail, tail_bytes, split_at as u64);
    (head, tail)
}

pub(super) fn json_response_bytes(content: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"model":"deepseek-chat","choices":[{{"message":{{"content":"{content}"}}}}]}}"#
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

pub(super) fn http_request_event_with(
    event_id: EventId,
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    sequence: u64,
) -> DomainEvent {
    let mut event = http_request_event(event_id, process);
    let EventPayload::Application(payload) = &mut event.payload else {
        unreachable!("http_request_event returns an application event");
    };
    payload.metadata.insert(
        "payload_segment_id".to_string(),
        segment_id.get().to_string(),
    );
    payload
        .metadata
        .insert("payload_sequence".to_string(), sequence.to_string());
    event
}

pub(super) fn raw_sse_response_bytes(content: &str) -> Vec<u8> {
    format!(
        "data: {{\"model\":\"deepseek-chat\",\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}\n\ndata: [DONE]\n\n"
    )
    .into_bytes()
}

pub(super) fn http2_llm_request_segment(
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    stream_id: u32,
) -> PayloadSegment {
    let body = br#"{"model":"deepseek-chat","messages":[{"role":"user","content":"hello"}]}"#;
    payload_segment_with_bytes(
        process,
        PayloadDirection::Outbound,
        segment_id,
        operation_id,
        sequence,
        http2_message_bytes(stream_id, body),
    )
}

pub(super) fn outbound_payload_segment_with_bytes(
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    bytes: Vec<u8>,
) -> PayloadSegment {
    payload_segment_with_bytes(
        process,
        PayloadDirection::Outbound,
        segment_id,
        operation_id,
        sequence,
        bytes,
    )
}

pub(super) fn http2_pending_request_head_segment(
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    stream_id: u32,
) -> PayloadSegment {
    payload_segment_with_bytes(
        process,
        PayloadDirection::Outbound,
        segment_id,
        operation_id,
        sequence,
        http2_frame(HTTP2_HEADERS_FRAME_TYPE, stream_id, b"\x82"),
    )
}

pub(super) fn http2_llm_response_segment(
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    stream_id: u32,
) -> PayloadSegment {
    let body = br#"{"model":"deepseek-chat","choices":[{"message":{"content":"first"}}]}"#;
    payload_segment_with_bytes(
        process,
        PayloadDirection::Inbound,
        segment_id,
        operation_id,
        sequence,
        http2_message_bytes(stream_id, body),
    )
}

fn payload_segment_with_bytes(
    process: ProcessIdentity,
    direction: PayloadDirection,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    bytes: Vec<u8>,
) -> PayloadSegment {
    let mut segment = llm_payload_segment(process);
    segment.direction = direction;
    segment.segment_id = segment_id;
    segment.operation_id = operation_id;
    segment.sequence = sequence;
    segment.protocol_hint = Some("h2".to_string());
    set_segment_bytes(&mut segment, bytes, 0);
    segment
}

fn set_segment_bytes(segment: &mut PayloadSegment, bytes: Vec<u8>, operation_offset: u64) {
    let size = bytes.len() as u64;
    segment.bytes = bytes;
    segment.original_size = size;
    segment.captured_size = size;
    segment.operation_offset = operation_offset;
    segment.operation_original_size = size;
    segment.operation_captured_size = size;
}

fn http2_message_bytes(stream_id: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = http2_frame(HTTP2_HEADERS_FRAME_TYPE, stream_id, b"\x82");
    bytes.extend(http2_frame(HTTP2_DATA_FRAME_TYPE, stream_id, body));
    bytes
}

fn http2_frame(frame_type: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= HTTP2_MAX_FRAME_LENGTH_FIELD);
    assert!(stream_id != 0);
    let length = payload.len();
    let mut frame = Vec::with_capacity(HTTP2_FRAME_HEADER_BYTES + length);
    frame.push(((length >> 16) & HTTP2_LENGTH_BYTE_MASK) as u8);
    frame.push(((length >> 8) & HTTP2_LENGTH_BYTE_MASK) as u8);
    frame.push((length & HTTP2_LENGTH_BYTE_MASK) as u8);
    frame.push(frame_type);
    frame.push(0);
    frame.extend_from_slice(&(stream_id & HTTP2_RESERVED_STREAM_ID_MASK).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

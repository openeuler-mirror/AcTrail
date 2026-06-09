use std::time::SystemTime;

use config_core::daemon::{ApplicationProtocolConfig, SseDataPolicy};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSegmentId, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use super::ApplicationProtocolAnalyzer;

const TEST_TRACE_ID: TraceId = TraceId::new(1);
const OTHER_TRACE_ID: TraceId = TraceId::new(2);
const TEST_PID: u32 = 1000;
const TEST_START_TICKS: u64 = 2000;
const TEST_GENERATION: u64 = 3000;
const TEST_REQUEST_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(10);
const TEST_BINARY_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(11);
const TEST_HTTP2_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(12);
const OTHER_TRACE_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(13);
const TEST_REQUEST_SEQUENCE: u64 = 0;
const TEST_BINARY_SEQUENCE: u64 = 1;
const TEST_HTTP2_SEQUENCE: u64 = 2;
const OTHER_TRACE_SEQUENCE: u64 = 3;
const TEST_BUFFER_BYTES: u64 = 4096;
const TEST_HTTP2_MAX_FRAME_BYTES: u64 = 16384;
const TEST_HTTP2_PREVIEW_BYTES: u64 = 16;
const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const HTTP2_DATA_FRAME_TYPE: u8 = 0;
const HTTP2_SETTINGS_FRAME_TYPE: u8 = 4;
const HTTP2_STREAM_ONE: u32 = 1;
const HTTP2_CONNECTION_STREAM: u32 = 0;
const HTTP2_BYTE_MASK: u32 = 0xff;

#[test]
fn http1_stream_prevents_later_binary_body_from_being_scanned_as_http2() {
    let binary = h2_like_binary_body();
    let mut unclassified = ApplicationProtocolAnalyzer::new(test_config());
    let h2_drafts = unclassified
        .analyze(&payload_segment(
            TEST_HTTP2_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Inbound,
            TEST_HTTP2_SEQUENCE,
            binary.clone(),
        ))
        .unwrap();
    assert!(h2_drafts.iter().any(|draft| draft.payload.protocol == "h2"));

    let mut analyzer = ApplicationProtocolAnalyzer::new(test_config());
    let request_drafts = analyzer
        .analyze(&payload_segment(
            TEST_REQUEST_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Outbound,
            TEST_REQUEST_SEQUENCE,
            b"GET /claude-code-releases/2.1.168/linux-x64/claude HTTP/1.1\r\nHost: downloads.claude.ai\r\n\r\n".to_vec(),
        ))
        .unwrap();
    assert!(request_drafts.iter().any(|draft| {
        draft.payload.protocol == "http/1.1" && draft.payload.operation == "request"
    }));

    let binary_drafts = analyzer
        .analyze(&payload_segment(
            TEST_BINARY_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Inbound,
            TEST_BINARY_SEQUENCE,
            binary,
        ))
        .unwrap();
    assert!(
        !binary_drafts
            .iter()
            .any(|draft| draft.payload.protocol == "h2")
    );
}

#[test]
fn h2_preface_is_not_locked_as_http1_fallback() {
    let mut analyzer = ApplicationProtocolAnalyzer::new(test_config());
    let preface_drafts = analyzer
        .analyze(&payload_segment(
            TEST_HTTP2_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Inbound,
            TEST_HTTP2_SEQUENCE,
            h2_preface_and_settings(),
        ))
        .unwrap();
    assert!(preface_drafts.iter().any(|draft| {
        draft.payload.protocol == "h2" && draft.payload.operation == "connection_preface"
    }));
    assert_eq!(analyzer.known_stream_protocol_count(), 1);

    let data_drafts = analyzer
        .analyze(&payload_segment(
            TEST_BINARY_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Inbound,
            TEST_BINARY_SEQUENCE,
            h2_like_binary_body(),
        ))
        .unwrap();
    assert!(
        data_drafts
            .iter()
            .any(|draft| draft.payload.protocol == "h2")
    );
}

#[test]
fn forget_trace_removes_only_matching_protocol_cache_entries() {
    let mut analyzer = ApplicationProtocolAnalyzer::new(test_config());
    analyzer
        .analyze(&payload_segment_on_stream(
            TEST_REQUEST_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Outbound,
            TEST_REQUEST_SEQUENCE,
            "http1-one",
            b"GET /partial HTTP/1.1\r\nHost: one.local\r\n".to_vec(),
        ))
        .unwrap();
    analyzer
        .analyze(&payload_segment_on_stream(
            OTHER_TRACE_SEGMENT_ID,
            OTHER_TRACE_ID,
            PayloadDirection::Outbound,
            OTHER_TRACE_SEQUENCE,
            "http1-two",
            b"GET /partial HTTP/1.1\r\nHost: two.local\r\n".to_vec(),
        ))
        .unwrap();
    assert_eq!(analyzer.buffered_http1_stream_count(), 2);

    analyzer
        .analyze(&payload_segment_on_stream(
            TEST_HTTP2_SEGMENT_ID,
            TEST_TRACE_ID,
            PayloadDirection::Inbound,
            TEST_HTTP2_SEQUENCE,
            "h2-one",
            h2_preface_and_settings(),
        ))
        .unwrap();
    analyzer
        .analyze(&payload_segment_on_stream(
            TEST_BINARY_SEGMENT_ID,
            OTHER_TRACE_ID,
            PayloadDirection::Inbound,
            TEST_BINARY_SEQUENCE,
            "h2-two",
            h2_preface_and_settings(),
        ))
        .unwrap();
    assert_eq!(analyzer.known_stream_protocol_count(), 2);

    analyzer.forget_trace(TEST_TRACE_ID);
    assert_eq!(analyzer.known_stream_protocol_count(), 1);
    assert_eq!(analyzer.buffered_http1_stream_count(), 1);
}

fn test_config() -> ApplicationProtocolConfig {
    ApplicationProtocolConfig {
        enabled: true,
        http1_enabled: true,
        http2_enabled: true,
        capture_host: true,
        sse_enabled: false,
        sse_data_policy: SseDataPolicy::Disabled,
        sse_max_buffer_bytes: TEST_BUFFER_BYTES,
        sse_max_data_bytes: TEST_BUFFER_BYTES,
        http2_max_frame_bytes: TEST_HTTP2_MAX_FRAME_BYTES,
        http2_max_connection_buffer_bytes: TEST_BUFFER_BYTES,
        http2_emit_data_preview: false,
        http2_max_data_preview_bytes: TEST_HTTP2_PREVIEW_BYTES,
    }
}

fn h2_like_binary_body() -> Vec<u8> {
    h2_frame(HTTP2_DATA_FRAME_TYPE, HTTP2_STREAM_ONE)
}

fn h2_preface_and_settings() -> Vec<u8> {
    let mut bytes = HTTP2_CONNECTION_PREFACE.to_vec();
    bytes.extend_from_slice(&h2_frame(
        HTTP2_SETTINGS_FRAME_TYPE,
        HTTP2_CONNECTION_STREAM,
    ));
    bytes
}

fn h2_frame(frame_type: u8, stream_id: u32) -> Vec<u8> {
    let length = 0_u32;
    vec![
        ((length >> 16) & HTTP2_BYTE_MASK) as u8,
        ((length >> 8) & HTTP2_BYTE_MASK) as u8,
        (length & HTTP2_BYTE_MASK) as u8,
        frame_type,
        0,
        ((stream_id >> 24) & HTTP2_BYTE_MASK) as u8,
        ((stream_id >> 16) & HTTP2_BYTE_MASK) as u8,
        ((stream_id >> 8) & HTTP2_BYTE_MASK) as u8,
        (stream_id & HTTP2_BYTE_MASK) as u8,
    ]
}

fn payload_segment(
    segment_id: PayloadSegmentId,
    trace_id: TraceId,
    direction: PayloadDirection,
    sequence: u64,
    bytes: Vec<u8>,
) -> PayloadSegment {
    payload_segment_on_stream(
        segment_id,
        trace_id,
        direction,
        sequence,
        "tls-stream",
        bytes,
    )
}

fn payload_segment_on_stream(
    segment_id: PayloadSegmentId,
    trace_id: TraceId,
    direction: PayloadDirection,
    sequence: u64,
    stream_key: &str,
    bytes: Vec<u8>,
) -> PayloadSegment {
    let size = bytes.len() as u64;
    PayloadSegment {
        segment_id,
        trace_id,
        observed_at: SystemTime::UNIX_EPOCH,
        process: ProcessIdentity::new(TEST_PID, TEST_START_TICKS, TEST_GENERATION),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new(stream_key),
        sequence,
        original_size: size,
        captured_size: size,
        operation_id: sequence,
        operation_offset: 0,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        redaction: PayloadRedactionState::Unredacted,
        library: "openssl".to_string(),
        symbol: "SSL_read".to_string(),
        protocol_hint: None,
        bytes,
    }
}

use std::time::SystemTime;

use config_core::daemon::{
    ApplicationProtocolConfig, Http2DataContentRetention, SemanticRetentionConfig, SseDataPolicy,
};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSegmentId, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use super::Http2Analyzer;
use super::frame::{CONNECTION_PREFACE, FRAME_HEADER_BYTES};

const TEST_MAX_FRAME_BYTES: u64 = 16384;
const TEST_MAX_BUFFER_BYTES: u64 = 1048576;
const TEST_PREVIEW_BYTES: u64 = 4096;
const SETTINGS_FRAME_TYPE: u8 = 4;
const DATA_FRAME_TYPE: u8 = 0;
const END_STREAM_FLAG: u8 = 1;
const STREAM_ONE: u32 = 1;
const TEST_TRACE_ID: TraceId = TraceId::new(1);
const OTHER_TRACE_ID: TraceId = TraceId::new(2);

#[test]
fn strips_inbound_connection_preface_before_frame_decode() {
    let mut analyzer = Http2Analyzer::new_with_retention(test_config(), preview_retention());
    let mut bytes = CONNECTION_PREFACE.to_vec();
    bytes.extend_from_slice(&test_frame(SETTINGS_FRAME_TYPE, 0, 0, b""));
    bytes.extend_from_slice(&test_frame(
        DATA_FRAME_TYPE,
        END_STREAM_FLAG,
        STREAM_ONE,
        b"{\"ok\":true}",
    ));

    let drafts = analyzer
        .analyze(&payload_segment(PayloadDirection::Inbound, bytes))
        .unwrap();

    assert!(
        drafts
            .iter()
            .any(|draft| draft.payload.operation == "connection_preface")
    );
    assert!(drafts.iter().any(|draft| {
        draft.payload.operation == "frame"
            && draft.payload.metadata.get("frame_type").map(String::as_str) == Some("SETTINGS")
    }));
    assert!(drafts.iter().any(|draft| {
        draft.payload.operation == "data"
            && draft
                .payload
                .metadata
                .get("data_preview")
                .map(String::as_str)
                == Some("{\"ok\":true}")
    }));
}

#[test]
fn oversized_known_frame_drops_direction_without_error() {
    let mut analyzer = Http2Analyzer::new_with_retention(test_config(), preview_retention());
    let oversized = test_frame_header(
        TEST_MAX_FRAME_BYTES + 1,
        DATA_FRAME_TYPE,
        END_STREAM_FLAG,
        STREAM_ONE,
    );

    let drafts = analyzer
        .analyze(&payload_segment(PayloadDirection::Inbound, oversized))
        .expect("oversized observed frame should not fail the daemon");
    assert!(drafts.is_empty());

    let valid = test_frame(DATA_FRAME_TYPE, END_STREAM_FLAG, STREAM_ONE, b"{}");
    let drafts = analyzer
        .analyze(&payload_segment(PayloadDirection::Inbound, valid))
        .expect("analyzer should accept later frames after dropping bad state");
    assert!(drafts.iter().any(|draft| {
        draft.payload.operation == "data"
            && draft
                .payload
                .metadata
                .get("data_preview")
                .map(String::as_str)
                == Some("{}")
    }));
}

#[test]
fn oversized_frame_does_not_drop_opposite_direction_buffer() {
    let mut analyzer = Http2Analyzer::new_with_retention(test_config(), preview_retention());
    let mut preface = CONNECTION_PREFACE.to_vec();
    preface.extend_from_slice(&test_frame(SETTINGS_FRAME_TYPE, 0, 0, b""));
    analyzer
        .analyze(&payload_segment(PayloadDirection::Inbound, preface))
        .unwrap();

    let valid = test_frame(DATA_FRAME_TYPE, END_STREAM_FLAG, STREAM_ONE, b"{}");
    let (first, second) = valid.split_at(FRAME_HEADER_BYTES + 1);
    let drafts = analyzer
        .analyze(&payload_segment(PayloadDirection::Inbound, first.to_vec()))
        .unwrap();
    assert!(drafts.is_empty());

    let oversized = test_frame_header(
        TEST_MAX_FRAME_BYTES + 1,
        DATA_FRAME_TYPE,
        END_STREAM_FLAG,
        STREAM_ONE,
    );
    let drafts = analyzer
        .analyze(&payload_segment(PayloadDirection::Outbound, oversized))
        .expect("oversized opposite-direction frame should not fail the daemon");
    assert!(drafts.is_empty());

    let drafts = analyzer
        .analyze(&payload_segment(PayloadDirection::Inbound, second.to_vec()))
        .unwrap();
    assert!(drafts.iter().any(|draft| {
        draft.payload.operation == "data"
            && draft
                .payload
                .metadata
                .get("data_preview")
                .map(String::as_str)
                == Some("{}")
    }));
}

#[test]
fn forget_trace_removes_confirmed_connections_for_matching_trace() {
    let mut analyzer = Http2Analyzer::new(test_config());
    analyzer
        .analyze(&payload_segment_for_trace(
            TEST_TRACE_ID,
            PayloadDirection::Inbound,
            preface_and_settings(),
        ))
        .unwrap();
    analyzer
        .analyze(&payload_segment_for_trace(
            OTHER_TRACE_ID,
            PayloadDirection::Inbound,
            preface_and_settings(),
        ))
        .unwrap();
    assert_eq!(analyzer.connection_count(), 2);

    analyzer.forget_trace(TEST_TRACE_ID);
    assert_eq!(analyzer.connection_count(), 1);
}

fn test_config() -> ApplicationProtocolConfig {
    ApplicationProtocolConfig {
        enabled: true,
        http1_enabled: false,
        http2_enabled: true,
        capture_host: false,
        sse_enabled: false,
        sse_data_policy: SseDataPolicy::Disabled,
        sse_max_buffer_bytes: TEST_MAX_BUFFER_BYTES,
        sse_max_data_bytes: TEST_PREVIEW_BYTES,
        http2_max_frame_bytes: TEST_MAX_FRAME_BYTES,
        http2_max_connection_buffer_bytes: TEST_MAX_BUFFER_BYTES,
        http2_emit_data_preview: true,
        http2_max_data_preview_bytes: TEST_PREVIEW_BYTES,
    }
}

fn preview_retention() -> SemanticRetentionConfig {
    let mut config = SemanticRetentionConfig::default();
    config.l3_http2_frame.data_content = Http2DataContentRetention::Preview;
    config
}

fn payload_segment(direction: PayloadDirection, bytes: Vec<u8>) -> PayloadSegment {
    payload_segment_for_trace(TEST_TRACE_ID, direction, bytes)
}

fn payload_segment_for_trace(
    trace_id: TraceId,
    direction: PayloadDirection,
    bytes: Vec<u8>,
) -> PayloadSegment {
    let size = bytes.len() as u64;
    PayloadSegment {
        segment_id: PayloadSegmentId::new(0),
        trace_id,
        observed_at: SystemTime::UNIX_EPOCH,
        process: ProcessIdentity::new(1),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new("h2-test"),
        sequence: 0,
        original_size: size,
        captured_size: size,
        operation_id: 0,
        operation_offset: 0,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        redaction: PayloadRedactionState::Unredacted,
        library: "openssl".to_string(),
        symbol: "SSL_read".to_string(),
        protocol_hint: Some("h2".to_string()),
        bytes,
    }
}

fn preface_and_settings() -> Vec<u8> {
    let mut bytes = CONNECTION_PREFACE.to_vec();
    bytes.extend_from_slice(&test_frame(SETTINGS_FRAME_TYPE, 0, 0, b""));
    bytes
}

fn test_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let length = payload.len() as u32;
    let mut bytes = test_frame_header(u64::from(length), frame_type, flags, stream_id);
    bytes.extend_from_slice(payload);
    bytes
}

fn test_frame_header(length: u64, frame_type: u8, flags: u8, stream_id: u32) -> Vec<u8> {
    let length = length as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&length.to_be_bytes()[1..]);
    bytes.push(frame_type);
    bytes.push(flags);
    bytes.extend_from_slice(&stream_id.to_be_bytes());
    bytes
}

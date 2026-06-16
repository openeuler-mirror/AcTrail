use super::*;
use config_core::daemon::SseDataPolicy;

const TEST_SSE_MAX_BUFFER_BYTES: u64 = 4096;
const TEST_SSE_MAX_DATA_BYTES: u64 = 4096;
const TEST_HTTP2_MAX_FRAME_BYTES: u64 = 16384;
const TEST_HTTP2_MAX_CONNECTION_BUFFER_BYTES: u64 = 4096;
const TEST_HTTP2_MAX_DATA_PREVIEW_BYTES: u64 = 4096;

#[test]
fn chunked_response_with_dechunked_sse_body_does_not_error_when_sse_is_disabled() {
    let mut text = claude_streaming_response_fragment();
    let message = take_message(&mut text, &test_config(false), false)
        .unwrap()
        .expect("HTTP response headers");

    assert_eq!(message.first_line, "HTTP/1.1 200 OK");
    assert!(message.body.is_empty());
    assert!(text.is_empty());
}

#[test]
fn chunked_response_with_dechunked_sse_body_can_emit_sse_preview() {
    let config = test_config(true);
    let mut text = claude_streaming_response_fragment();
    let message = take_message(&mut text, &config, false)
        .unwrap()
        .expect("HTTP response headers");

    let events = message.sse_events(&config).unwrap();
    assert!(events
        .iter()
        .any(|payload| payload.operation == "event" && payload.summary == "content_block_delta"));
    assert!(text.is_empty());
}

fn claude_streaming_response_fragment() -> String {
    concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Transfer-Encoding: chunked\r\n",
        "\r\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0}\n\n",
    )
    .to_string()
}

fn test_config(sse_enabled: bool) -> ApplicationProtocolConfig {
    ApplicationProtocolConfig {
        enabled: true,
        http1_enabled: true,
        http2_enabled: false,
        capture_host: false,
        sse_enabled,
        sse_data_policy: if sse_enabled {
            SseDataPolicy::Preview
        } else {
            SseDataPolicy::Disabled
        },
        sse_max_buffer_bytes: TEST_SSE_MAX_BUFFER_BYTES,
        sse_max_data_bytes: TEST_SSE_MAX_DATA_BYTES,
        http2_max_frame_bytes: TEST_HTTP2_MAX_FRAME_BYTES,
        http2_max_connection_buffer_bytes: TEST_HTTP2_MAX_CONNECTION_BUFFER_BYTES,
        http2_emit_data_preview: false,
        http2_max_data_preview_bytes: TEST_HTTP2_MAX_DATA_PREVIEW_BYTES,
    }
}

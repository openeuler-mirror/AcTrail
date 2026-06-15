use std::time::SystemTime;

use config_core::daemon::{ApplicationProtocolConfig, DiagnosticLogLevel, SseDataPolicy};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::event::EventPayload;
use model_core::ids::{ProfileName, TraceName};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use storage_core::PayloadSegmentQuery;

use crate::profiles::DaemonProfileRegistry;

#[test]
fn socket_payload_gate_persists_http_and_drops_non_http_bytes() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-application-protocol-socket-http-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut payload_config = super::super::payload_config(false);
    payload_config.socket.enabled = true;
    let mut wiring = super::super::super::build_runtime_wiring(
        &super::super::test_storage_config(storage_path.clone()),
        profiles,
        super::super::ebpf_config(false),
        payload_config,
        DiagnosticLogLevel::Info,
        super::super::seccomp_notify_disabled(),
        super::super::process_seccomp_disabled(),
        super::super::agent_invocation_disabled(),
        ApplicationProtocolConfig {
            enabled: true,
            http1_enabled: true,
            http2_enabled: false,
            capture_host: true,
            sse_enabled: false,
            sse_data_policy: SseDataPolicy::Disabled,
            sse_max_buffer_bytes: super::super::TEST_HTTP_BUFFER_BYTES,
            sse_max_data_bytes: super::super::TEST_HTTP_BUFFER_BYTES,
            http2_max_frame_bytes: super::super::TEST_HTTP2_MAX_FRAME_BYTES,
            http2_max_connection_buffer_bytes: super::super::TEST_HTTP_BUFFER_BYTES,
            http2_emit_data_preview: false,
            http2_max_data_preview_bytes: super::super::TEST_HTTP2_PREVIEW_BYTES,
        },
        super::super::resource_metrics_disabled(),
        super::super::export_runtime_disabled(),
        super::super::enforcement_disabled(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(std::process::id(), 1, 1);
    super::super::create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("application-socket-http"),
        TraceName::new("application-socket-http"),
        vec![
            CapabilityRequest::new(Capability::SocketPlaintextPayload, RequestMode::Required),
            CapabilityRequest::new(
                Capability::NetApplicationPlaintextHttp,
                RequestMode::Required,
            ),
        ],
        super::super::APPLICATION_PROTOCOL_COLLECTOR,
        vec![Capability::NetApplicationPlaintextHttp],
    );

    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_socket_segment(
                trace_id,
                process.clone(),
                "socket-non-http",
                PayloadDirection::Outbound,
                0,
                b"not an HTTP protocol message\n".to_vec(),
            ),
        )
        .unwrap();
    assert!(
        wiring
            .attach_service
            .storage
            .list_payload_segments(
                trace_id,
                PayloadSegmentQuery {
                    segment_id: None,
                    direction: None,
                    limit: None,
                    include_bytes: true,
                },
            )
            .unwrap()
            .is_empty()
    );

    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_socket_segment(
                trace_id,
                process,
                "socket-http",
                PayloadDirection::Outbound,
                1,
                b"GET /plain HTTP/1.1\r\nHost: local.test\r\n\r\n".to_vec(),
            ),
        )
        .unwrap();

    let payloads = wiring
        .attach_service
        .storage
        .list_payload_segments(
            trace_id,
            PayloadSegmentQuery {
                segment_id: None,
                direction: Some(PayloadDirection::Outbound),
                limit: None,
                include_bytes: true,
            },
        )
        .unwrap();
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].source_boundary, PayloadSourceBoundary::Syscall);
    assert_eq!(payloads[0].library, "socket-syscall");
    assert_eq!(payloads[0].protocol_hint.as_deref(), Some("http/1.x"));

    let events = wiring.attach_service.storage.list_events(trace_id).unwrap();
    assert!(events.iter().any(|event| match &event.payload {
        EventPayload::Application(payload) =>
            payload.operation == "request"
                && payload.summary == "GET /plain"
                && payload.metadata.get("host").map(String::as_str) == Some("local.test"),
        _ => false,
    }));
}

fn raw_socket_segment(
    trace_id: model_core::ids::TraceId,
    process: ProcessIdentity,
    stream_key: &str,
    direction: PayloadDirection,
    sequence: u64,
    bytes: Vec<u8>,
) -> RawPayloadSegment {
    RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::UNIX_EPOCH,
        process,
        source_boundary: PayloadSourceBoundary::Syscall,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new(stream_key),
        sequence,
        original_size: bytes.len() as u64,
        captured_size: bytes.len() as u64,
        operation_id: sequence,
        operation_offset: 0,
        operation_original_size: bytes.len() as u64,
        operation_captured_size: bytes.len() as u64,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        library: "socket-syscall".to_string(),
        symbol: "sendto".to_string(),
        protocol_hint: None,
        bytes,
    }
}

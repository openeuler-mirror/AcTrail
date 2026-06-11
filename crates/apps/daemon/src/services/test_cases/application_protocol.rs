use std::time::SystemTime;

use config_core::daemon::{
    ApplicationProtocolConfig, DEFAULT_STORAGE_BUSY_TIMEOUT_MS, DiagnosticLogLevel,
    PayloadRedactionPolicy, SseDataPolicy,
};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::event::EventPayload;
use model_core::ids::{ProfileName, TraceName};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use semantic_action::{SemanticActionKind, SemanticActionReadStore};
use store_read_contract::events::EventReadStore;
use store_read_contract::payloads::{PayloadReadStore, PayloadSegmentQuery};

use crate::profiles::DaemonProfileRegistry;

#[path = "application_protocol_socket.rs"]
mod socket;

const TEST_SSE_PREVIEW_BYTES: u64 = 8;
const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const HTTP2_SETTINGS_FRAME_TYPE: u8 = 4;
const HTTP2_DATA_FRAME_TYPE: u8 = 0;
const HTTP2_END_STREAM_FLAG: u8 = 1;
const HTTP2_STREAM_ONE: u32 = 1;
const HTTP2_CONNECTION_STREAM: u32 = 0;
const HTTP2_RESERVED_STREAM_ID_MASK: u32 = 0x7fff_ffff;
const HTTP2_BYTE_MASK: u32 = 0xff;

#[test]
fn tls_payload_processing_persists_http_and_sse_application_events() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-application-protocol-http1-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut payload_config = super::payload_config(true);
    payload_config.tls.redaction_policy = PayloadRedactionPolicy::AuthorizationHeader;
    let mut wiring = super::super::build_runtime_wiring(
        &storage_path,
        DEFAULT_STORAGE_BUSY_TIMEOUT_MS,
        profiles,
        super::ebpf_config(false),
        payload_config,
        DiagnosticLogLevel::Info,
        super::seccomp_notify_disabled(),
        super::process_seccomp_disabled(),
        super::agent_invocation_disabled(),
        ApplicationProtocolConfig {
            enabled: true,
            http1_enabled: true,
            http2_enabled: false,
            capture_host: true,
            sse_enabled: true,
            sse_data_policy: SseDataPolicy::Preview,
            sse_max_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            sse_max_data_bytes: TEST_SSE_PREVIEW_BYTES,
            http2_max_frame_bytes: super::TEST_HTTP2_MAX_FRAME_BYTES,
            http2_max_connection_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_emit_data_preview: false,
            http2_max_data_preview_bytes: super::TEST_HTTP2_PREVIEW_BYTES,
        },
        super::resource_metrics_disabled(),
        super::live_otel_export_disabled(),
        super::enforcement_disabled(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(std::process::id(), 1, 1);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("application-http"),
        TraceName::new("application-http"),
        vec![
            CapabilityRequest::new(Capability::TlsPlaintextPayload, RequestMode::Required),
            CapabilityRequest::new(
                Capability::NetApplicationPlaintextHttp,
                RequestMode::Required,
            ),
        ],
        super::APPLICATION_PROTOCOL_COLLECTOR,
        vec![Capability::NetApplicationPlaintextHttp],
    );

    let request_body =
        r#"{"model":"test-model","messages":[{"role":"user","content":"hello"}],"stream":true}"#;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: api.local\r\nAuthorization: Bearer secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        request_body.len(),
        request_body
    );
    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_tls_segment(
                trace_id,
                process.clone(),
                PayloadDirection::Outbound,
                0,
                request.into_bytes(),
            ),
        )
        .unwrap();

    let sse_body = concat!(
        "event: token\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        sse_body.len(),
        sse_body
    );
    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_tls_segment(
                trace_id,
                process,
                PayloadDirection::Inbound,
                1,
                response.into_bytes(),
            ),
        )
        .unwrap();

    let events = wiring.attach_service.storage.list_events(trace_id).unwrap();
    let applications = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Application(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(applications.iter().any(|payload| {
        payload.operation == "request"
            && payload.summary == "POST /v1/chat/completions"
            && payload.metadata.get("host").map(String::as_str) == Some("api.local")
    }));
    assert!(applications.iter().any(|payload| {
        payload.operation == "response"
            && payload.summary == "200 OK"
            && payload.metadata.get("content_type").map(String::as_str) == Some("text/event-stream")
    }));
    assert!(applications.iter().any(|payload| {
        payload.protocol == "sse"
            && payload.operation == "event"
            && payload.summary == "token"
            && payload.metadata.get("data_truncated").map(String::as_str) == Some("true")
    }));
    let actions = wiring
        .attach_service
        .storage
        .list_semantic_actions(trace_id)
        .unwrap();
    let response = actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("inbound SSE payload should persist an llm.response action");
    assert_eq!(
        response
            .attributes
            .get("llm.response.output_text")
            .map(String::as_str),
        Some("ok")
    );
    assert!(response.evidence.iter().any(|evidence| {
        evidence.role == "llm.response.payload"
            && payloads_contain_id(&wiring.attach_service.storage, trace_id, evidence.id)
    }));

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
    let request_text = String::from_utf8(payloads[0].bytes.clone()).unwrap();
    assert!(request_text.contains("Authorization: <redacted>"));
    assert!(!request_text.contains("Bearer secret"));
}

fn payloads_contain_id(
    storage: &sqlite_storage::SqliteStorage,
    trace_id: model_core::ids::TraceId,
    evidence_id: u64,
) -> bool {
    storage
        .list_payload_segments(
            trace_id,
            PayloadSegmentQuery {
                segment_id: None,
                direction: None,
                limit: None,
                include_bytes: false,
            },
        )
        .unwrap()
        .iter()
        .any(|segment| segment.segment_id.get() == evidence_id)
}

#[test]
fn tls_payload_processing_persists_http2_frame_and_data_events() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-application-protocol-http2-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut wiring = super::super::build_runtime_wiring(
        &storage_path,
        DEFAULT_STORAGE_BUSY_TIMEOUT_MS,
        profiles,
        super::ebpf_config(false),
        super::payload_config(true),
        DiagnosticLogLevel::Info,
        super::seccomp_notify_disabled(),
        super::process_seccomp_disabled(),
        super::agent_invocation_disabled(),
        ApplicationProtocolConfig {
            enabled: true,
            http1_enabled: false,
            http2_enabled: true,
            capture_host: false,
            sse_enabled: false,
            sse_data_policy: SseDataPolicy::Disabled,
            sse_max_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            sse_max_data_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_max_frame_bytes: super::TEST_HTTP2_MAX_FRAME_BYTES,
            http2_max_connection_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_emit_data_preview: true,
            http2_max_data_preview_bytes: super::TEST_HTTP_BUFFER_BYTES,
        },
        super::resource_metrics_disabled(),
        super::live_otel_export_disabled(),
        super::enforcement_disabled(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(std::process::id(), 1, 1);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("application-http2"),
        TraceName::new("application-http2"),
        vec![
            CapabilityRequest::new(Capability::TlsPlaintextPayload, RequestMode::Required),
            CapabilityRequest::new(Capability::NetApplicationHttp2Frames, RequestMode::Required),
        ],
        super::APPLICATION_PROTOCOL_COLLECTOR,
        vec![Capability::NetApplicationHttp2Frames],
    );

    let mut bytes = HTTP2_CONNECTION_PREFACE.to_vec();
    bytes.extend_from_slice(&http2_frame(
        HTTP2_SETTINGS_FRAME_TYPE,
        0,
        HTTP2_CONNECTION_STREAM,
        b"",
    ));
    bytes.extend_from_slice(&http2_frame(
        HTTP2_DATA_FRAME_TYPE,
        HTTP2_END_STREAM_FLAG,
        HTTP2_STREAM_ONE,
        b"{\"ok\":true}",
    ));
    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_tls_segment(trace_id, process, PayloadDirection::Outbound, 0, bytes),
        )
        .unwrap();

    let events = wiring.attach_service.storage.list_events(trace_id).unwrap();
    let applications = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Application(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        applications.iter().any(|payload| {
            payload.protocol == "h2" && payload.operation == "connection_preface"
        })
    );
    assert!(applications.iter().any(|payload| {
        payload.protocol == "h2"
            && payload.operation == "frame"
            && payload.metadata.get("frame_type").map(String::as_str) == Some("SETTINGS")
    }));
    assert!(applications.iter().any(|payload| {
        payload.protocol == "h2"
            && payload.operation == "data"
            && payload.metadata.get("stream_id").map(String::as_str) == Some("1")
            && payload.metadata.get("data_preview").map(String::as_str) == Some("{\"ok\":true}")
    }));
}

#[test]
fn http2_analyzer_ignores_http1_text_when_both_protocols_are_enabled() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-application-protocol-mixed-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut wiring = super::super::build_runtime_wiring(
        &storage_path,
        DEFAULT_STORAGE_BUSY_TIMEOUT_MS,
        profiles,
        super::ebpf_config(false),
        super::payload_config(true),
        DiagnosticLogLevel::Info,
        super::seccomp_notify_disabled(),
        super::process_seccomp_disabled(),
        super::agent_invocation_disabled(),
        ApplicationProtocolConfig {
            enabled: true,
            http1_enabled: true,
            http2_enabled: true,
            capture_host: true,
            sse_enabled: false,
            sse_data_policy: SseDataPolicy::Disabled,
            sse_max_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            sse_max_data_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_max_frame_bytes: super::TEST_HTTP2_MAX_FRAME_BYTES,
            http2_max_connection_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_emit_data_preview: false,
            http2_max_data_preview_bytes: super::TEST_HTTP2_PREVIEW_BYTES,
        },
        super::resource_metrics_disabled(),
        super::live_otel_export_disabled(),
        super::enforcement_disabled(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(std::process::id(), 1, 1);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("application-mixed"),
        TraceName::new("application-mixed"),
        vec![
            CapabilityRequest::new(Capability::TlsPlaintextPayload, RequestMode::Required),
            CapabilityRequest::new(
                Capability::NetApplicationPlaintextHttp,
                RequestMode::Required,
            ),
            CapabilityRequest::new(Capability::NetApplicationHttp2Frames, RequestMode::Required),
        ],
        super::APPLICATION_PROTOCOL_COLLECTOR,
        vec![
            Capability::NetApplicationPlaintextHttp,
            Capability::NetApplicationHttp2Frames,
        ],
    );

    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_tls_segment(
                trace_id,
                process,
                PayloadDirection::Outbound,
                0,
                b"GET /health HTTP/1.1\r\nHost: api.local\r\n\r\n".to_vec(),
            ),
        )
        .unwrap();

    let events = wiring.attach_service.storage.list_events(trace_id).unwrap();
    let applications = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Application(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        applications
            .iter()
            .any(|payload| { payload.protocol == "http/1.1" && payload.operation == "request" })
    );
    assert!(!applications.iter().any(|payload| payload.protocol == "h2"));
}

fn raw_tls_segment(
    trace_id: model_core::ids::TraceId,
    process: ProcessIdentity,
    direction: PayloadDirection,
    sequence: u64,
    bytes: Vec<u8>,
) -> RawPayloadSegment {
    RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::UNIX_EPOCH,
        process,
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new("tls-stream"),
        sequence,
        original_size: bytes.len() as u64,
        captured_size: bytes.len() as u64,
        operation_id: sequence,
        operation_offset: 0,
        operation_original_size: bytes.len() as u64,
        operation_captured_size: bytes.len() as u64,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        library: "openssl".to_string(),
        symbol: "SSL_write".to_string(),
        protocol_hint: Some("http/1.1".to_string()),
        bytes,
    }
}

fn http2_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).unwrap();
    let stream_id = stream_id & HTTP2_RESERVED_STREAM_ID_MASK;
    let mut frame = vec![
        ((length >> 16) & HTTP2_BYTE_MASK) as u8,
        ((length >> 8) & HTTP2_BYTE_MASK) as u8,
        (length & HTTP2_BYTE_MASK) as u8,
        frame_type,
        flags,
        ((stream_id >> 24) & HTTP2_BYTE_MASK) as u8,
        ((stream_id >> 16) & HTTP2_BYTE_MASK) as u8,
        ((stream_id >> 8) & HTTP2_BYTE_MASK) as u8,
        (stream_id & HTTP2_BYTE_MASK) as u8,
    ];
    frame.extend_from_slice(payload);
    frame
}

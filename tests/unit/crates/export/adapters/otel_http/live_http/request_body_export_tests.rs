// Tests for the corresponding production module.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, UNIX_EPOCH};

use export_core::SemanticActionKindSelection;
use model_core::ids::{OtelTraceId, ProfileName, TraceId, TraceName};
use model_core::process::ProcessIdentity;
use model_core::trace::{TraceAlertToken, TraceRecord};
use plugin_system::{ObservationBatch, ObservationConsumer};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys::{agent, llm_request, llm_tool_result, process},
};

use super::{build_otel_http_observation_consumer, metadata_only_action};
use crate::config::{
    OtelAttributeMode, OtelCompression, OtelEncoding, OtelHttpExporterConfig, OtelHttpTlsConfig,
};

fn test_config(
    endpoint: String,
    attribute_mode: OtelAttributeMode,
    action_kind: SemanticActionKind,
) -> OtelHttpExporterConfig {
    OtelHttpExporterConfig {
        endpoint,
        allow_insecure: true,
        queue_capacity: 16,
        batch_max_spans: 1,
        batch_timeout_ms: 60_000,
        connect_timeout_ms: 1_000,
        request_timeout_ms: 1_000,
        retry_max_attempts: 2,
        retry_backoff_ms: 10,
        shutdown_flush_deadline_ms: 500,
        tls: OtelHttpTlsConfig::default(),
        encoding: OtelEncoding::Json,
        compression: OtelCompression::None,
        headers: Vec::new(),
        action_kinds: SemanticActionKindSelection::from_config_entries([
            ("default".to_string(), false),
            (action_kind.as_str().to_string(), true),
        ])
        .expect("semantic action export policy"),
        attribute_mode,
    }
}

fn spawn_stub_collector() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub collector");
    let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set collector read timeout");
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(read) = socket.read(&mut buffer) {
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request_complete(&request) {
                break;
            }
        }
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let _ = sender.send(String::from_utf8_lossy(&request).to_string());
    });
    (endpoint, receiver)
}

fn request_complete(request: &[u8]) -> bool {
    let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    request.len() >= header_end + 4 + content_length
}

fn export_canonical_request_body(attribute_mode: OtelAttributeMode) -> String {
    let canonical_body = r#"{"messages":[{"content":"body-export-marker","role":"user"}]}"#;
    let action = SemanticAction {
        action_id: "llm-request-body".to_string(),
        trace_id: TraceId::new(7),
        kind: SemanticActionKind::LlmRequest,
        title: "LLM request".to_string(),
        start_time: UNIX_EPOCH,
        end_time: Some(UNIX_EPOCH + Duration::from_millis(1)),
        process: ProcessIdentity::new(100),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        attributes: BTreeMap::from([(
            llm_request::CANONICAL_BODY_JSON.to_string(),
            canonical_body.to_string(),
        )]),
        evidence: Vec::new(),
    };
    export_action(action, attribute_mode)
}

fn export_action(action: SemanticAction, attribute_mode: OtelAttributeMode) -> String {
    let (endpoint, received) = spawn_stub_collector();
    let consumer =
        build_otel_http_observation_consumer(test_config(endpoint, attribute_mode, action.kind))
            .expect("build consumer");
    let trace = TraceRecord::new(
        TraceId::new(7),
        OtelTraceId::from_bytes([7; OtelTraceId::BYTE_COUNT]).expect("non-zero OTEL trace ID"),
        TraceAlertToken::new([1; 32]),
        ProcessIdentity::new(100),
        TraceName::new("otel-http-content-export"),
        ProfileName::new("test"),
        UNIX_EPOCH,
    );

    consumer
        .consume(ObservationBatch {
            trace: &trace,
            trace_finalized: false,
            semantic_actions: std::slice::from_ref(&action),
            semantic_links: &[],
            file_observation_paths: &[],
            payload_refs: &[],
        })
        .expect("consume semantic action");

    received
        .recv_timeout(Duration::from_secs(2))
        .expect("collector receives the semantic action")
}

#[test]
fn full_attribute_mode_delivers_canonical_request_body_to_collector() {
    let request = export_canonical_request_body(OtelAttributeMode::Full);

    assert!(request.contains(llm_request::CANONICAL_BODY_JSON));
    assert!(request.contains("body-export-marker"));
}

#[test]
fn metadata_only_mode_keeps_canonical_request_body_out_of_collector() {
    let request = export_canonical_request_body(OtelAttributeMode::MetadataOnly);

    assert!(request.contains("llm.request"));
    assert!(!request.contains(llm_request::CANONICAL_BODY_JSON));
    assert!(!request.contains("body-export-marker"));
}

#[test]
fn metadata_only_mode_strips_opted_in_tool_result_content() {
    let action = SemanticAction {
        action_id: "tool-result".to_string(),
        trace_id: TraceId::new(7),
        kind: SemanticActionKind::LlmToolResult,
        title: "secret tool result".to_string(),
        start_time: UNIX_EPOCH,
        end_time: Some(UNIX_EPOCH),
        process: ProcessIdentity::new(100),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        attributes: BTreeMap::from([(
            llm_tool_result::CONTENT_JSON.to_string(),
            r#"{"stdout":"secret"}"#.to_string(),
        )]),
        evidence: Vec::new(),
    };

    let sanitized = metadata_only_action(&action);

    assert_eq!(sanitized.title, "llm.tool_result");
    assert!(
        !sanitized
            .attributes
            .contains_key(llm_tool_result::CONTENT_JSON)
    );
}

#[test]
fn metadata_only_mode_keeps_agent_identity_correlation_fields() {
    let action = SemanticAction {
        action_id: "agent-identity".to_string(),
        trace_id: TraceId::new(7),
        kind: SemanticActionKind::AgentIdentity,
        title: "agent identity process-100".to_string(),
        start_time: UNIX_EPOCH,
        end_time: Some(UNIX_EPOCH),
        process: ProcessIdentity::new(100),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        attributes: BTreeMap::from([
            (agent::IDENTITY_STATUS.to_string(), "observed".to_string()),
            (
                agent::IDENTITY_SOURCE.to_string(),
                "llm.request".to_string(),
            ),
            (
                agent::IDENTITY_EVIDENCE_ACTION_ID.to_string(),
                "llm-request-7".to_string(),
            ),
            (
                process::EXECUTABLE.to_string(),
                "/root/.cargo/bin/xiaoo".to_string(),
            ),
            (agent::TYPE.to_string(), "xiaoo".to_string()),
            (agent::TYPE_SOURCE.to_string(), "known_product".to_string()),
            ("secret.content".to_string(), "do not export".to_string()),
        ]),
        evidence: Vec::new(),
    };

    let sanitized = metadata_only_action(&action);

    assert_eq!(sanitized.title, "agent.identity");
    assert_eq!(
        sanitized
            .attributes
            .get(process::EXECUTABLE)
            .map(String::as_str),
        Some("/root/.cargo/bin/xiaoo")
    );
    assert_eq!(
        sanitized
            .attributes
            .get(agent::IDENTITY_STATUS)
            .map(String::as_str),
        Some("observed")
    );
    assert_eq!(
        sanitized
            .attributes
            .get(agent::IDENTITY_SOURCE)
            .map(String::as_str),
        Some("llm.request")
    );
    assert_eq!(
        sanitized
            .attributes
            .get(agent::IDENTITY_EVIDENCE_ACTION_ID)
            .map(String::as_str),
        Some("llm-request-7")
    );
    assert_eq!(
        sanitized.attributes.get(agent::TYPE).map(String::as_str),
        Some("xiaoo")
    );
    assert_eq!(
        sanitized
            .attributes
            .get(agent::TYPE_SOURCE)
            .map(String::as_str),
        Some("known_product")
    );
    assert!(!sanitized.attributes.contains_key("secret.content"));
}

#[test]
fn full_attribute_mode_delivers_opted_in_tool_result_content() {
    let action = SemanticAction {
        action_id: "tool-result".to_string(),
        trace_id: TraceId::new(7),
        kind: SemanticActionKind::LlmToolResult,
        title: "tool result".to_string(),
        start_time: UNIX_EPOCH,
        end_time: Some(UNIX_EPOCH),
        process: ProcessIdentity::new(100),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        attributes: BTreeMap::from([(
            llm_tool_result::CONTENT_JSON.to_string(),
            r#"{"stdout":"tool-result-marker"}"#.to_string(),
        )]),
        evidence: Vec::new(),
    };

    let request = export_action(action, OtelAttributeMode::Full);

    assert!(request.contains(llm_tool_result::CONTENT_JSON));
    assert!(request.contains("tool-result-marker"));
}

use config_core::daemon::{
    AgentInvocationConfig, SemanticContentOwner, SemanticRetentionConfig, SseEventContentRetention,
};
use model_core::payload::PayloadSegmentId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus,
    SemanticEvidenceKind,
};
use serde_json::Value;

use super::LiveSemanticActionRuntime;
use super::test_support::*;

const STRUCTURED_JSON_SSE_RESPONSE: &str =
    include_str!("fixtures/structured_json_sse_response.sse");
const STRUCTURED_REQUEST_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(500);
const STRUCTURED_RESPONSE_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(501);
const STRUCTURED_NON_LLM_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(502);
const STRUCTURED_REQUEST_OPERATION_ID: u64 = 500;
const STRUCTURED_RESPONSE_OPERATION_ID: u64 = 501;
const STRUCTURED_NON_LLM_OPERATION_ID: u64 = 502;
const STRUCTURED_REQUEST_SEQUENCE: u64 = 500;
const STRUCTURED_RESPONSE_SEQUENCE: u64 = 501;
const STRUCTURED_NON_LLM_SEQUENCE: u64 = 502;

#[test]
fn structured_json_sse_request_emits_llm_request_with_classifier_metadata() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        STRUCTURED_REQUEST_SEGMENT_ID,
        STRUCTURED_REQUEST_OPERATION_ID,
        STRUCTURED_REQUEST_SEQUENCE,
        structured_json_sse_relaxed_request_http_bytes(),
    ));

    let request = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("structured JSON/SSE request fixture should emit llm.request");
    assert_eq!(
        request
            .attributes
            .get("llm.request.classifier_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.protocol_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.model")
            .map(String::as_str),
        Some("glm-5.1")
    );
    assert_eq!(
        request.attributes.get("url.path").map(String::as_str),
        Some("/v1/structured/stream")
    );
}

#[test]
fn structured_json_sse_response_fixture_emits_llm_response_and_sse_stream() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        STRUCTURED_RESPONSE_SEGMENT_ID,
        STRUCTURED_RESPONSE_OPERATION_ID,
        STRUCTURED_RESPONSE_SEQUENCE,
        structured_json_sse_response_http_bytes(),
    ));

    let response = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("structured JSON/SSE response fixture should emit llm.response");
    assert_eq!(
        response
            .attributes
            .get("llm.response.provider_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.model")
            .map(String::as_str),
        Some("glm-5.1")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.content_text")
            .map(String::as_str),
        Some("Hello!")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.prompt_tokens")
            .map(String::as_str),
        Some("10")
    );

    let stream = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::SseStream)
        .expect("structured JSON/SSE response fixture should emit sse.stream");
    assert_eq!(
        stream.attributes.get("sse.event_count").map(String::as_str),
        Some("7")
    );
    assert_eq!(
        stream
            .attributes
            .get("sse.content_delta_count")
            .map(String::as_str),
        Some("2")
    );
}

#[test]
fn structured_json_sse_request_response_updates_one_llm_call() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let request_output =
        runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
            agent.clone(),
            STRUCTURED_REQUEST_SEGMENT_ID,
            STRUCTURED_REQUEST_OPERATION_ID,
            STRUCTURED_REQUEST_SEQUENCE,
            structured_json_sse_relaxed_request_http_bytes(),
        ));
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("structured request should emit llm.request");
    let request_call = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("structured request should emit llm.call");

    let response_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        STRUCTURED_RESPONSE_SEGMENT_ID,
        STRUCTURED_RESPONSE_OPERATION_ID,
        STRUCTURED_RESPONSE_SEQUENCE,
        structured_json_sse_response_http_bytes(),
    ));

    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("structured response should emit llm.response");
    let response_call = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("structured response should update llm.call");
    assert_eq!(response_call.action_id, request_call.action_id);
    assert_eq!(
        response_call
            .attributes
            .get("llm.call.request_action_id")
            .map(String::as_str),
        Some(request.action_id.as_str())
    );
    assert_eq!(
        response_call
            .attributes
            .get("llm.call.response_action_id")
            .map(String::as_str),
        Some(response.action_id.as_str())
    );
    assert!(request_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallRequest
            && link.parent_action_id == request_call.action_id
            && link.child_action_id == request.action_id
    }));
    assert!(response_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallResponse
            && link.parent_action_id == response_call.action_id
            && link.child_action_id == response.action_id
    }));
}

#[test]
fn structured_json_sse_request_with_string_message_content_emits_llm_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let request = structured_json_sse_relaxed_request_http_bytes();

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        STRUCTURED_REQUEST_SEGMENT_ID,
        STRUCTURED_REQUEST_OPERATION_ID,
        STRUCTURED_REQUEST_SEQUENCE,
        request,
    ));

    let request = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("structured JSON request shape should emit llm.request");
    assert_eq!(
        request
            .attributes
            .get("llm.request.classifier_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.protocol_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.model")
            .map(String::as_str),
        Some("glm-5.1")
    );
}

#[test]
fn structured_json_sse_chunked_request_split_across_segments_emits_llm_request_and_call() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = structured_json_sse_request_body();
    let request_head = format!(
        "POST /v1/structured/stream HTTP/1.1\r\nHost: llm.example.test\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        body.len()
    )
    .into_bytes();
    let request_segments = [
        request_head,
        body.as_bytes().to_vec(),
        b"\r\n".to_vec(),
        b"0\r\n\r\n".to_vec(),
    ];

    let mut final_output = None;
    for (index, bytes) in request_segments.into_iter().enumerate() {
        let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
            agent.clone(),
            response_segment_id(80 + index),
            response_operation_id(80 + index),
            response_sequence(80 + index),
            bytes,
        ));
        final_output = Some(output);
    }
    let final_output = final_output.expect("request segments should be observed");
    let request = final_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("complete chunked structured request should emit llm.request");
    assert_eq!(
        request
            .attributes
            .get("llm.request.protocol_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.model")
            .map(String::as_str),
        Some("glm-5.1")
    );
    let call = final_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("complete chunked structured request should open llm.call");
    assert_eq!(call.status, SemanticActionStatus::InProgress);
    assert_eq!(
        call.attributes
            .get("llm.call.request_action_id")
            .map(String::as_str),
        Some(request.action_id.as_str())
    );
}

#[test]
fn non_llm_chunked_request_emits_no_llm_actions() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = r#"{"function":"chat","messages":[{"role":"user","content":"hello"}]}"#;
    let bytes = format!(
        "POST /api/config HTTP/1.1\r\nHost: api.enterprise.trae.cn\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        body.len(),
        body
    )
    .into_bytes();

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        response_segment_id(90),
        response_operation_id(90),
        response_sequence(90),
        bytes,
    ));

    assert!(output.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmRequest
            && action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::LlmCall
    }));
}

#[test]
fn structured_json_sse_model_field_messages_without_context_emits_llm_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = concat!(
        "{\"messages\":[",
        "{\"role\":\"user\",\"content\":\"hello\"}",
        "],\"model\":\"glm-5.1\"}"
    );

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        response_segment_id(94),
        response_operation_id(94),
        response_sequence(94),
        http1_json_request_bytes("/v1/messages", "api.example.test", body),
    ));

    let request = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("model/messages structured request should emit llm.request");
    assert_eq!(
        request
            .attributes
            .get("llm.request.classifier_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.protocol_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.model")
            .map(String::as_str),
        Some("glm-5.1")
    );
}

#[test]
fn structured_json_sse_body_only_model_name_messages_with_context_emits_llm_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = structured_json_sse_request_body();

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        response_segment_id(95),
        response_operation_id(95),
        response_sequence(95),
        http2_request_bytes(7, body.as_bytes()),
    ));

    let request = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("body-only model_name/messages structured request should emit llm.request");
    assert_eq!(
        request
            .attributes
            .get("llm.request.protocol_id")
            .map(String::as_str),
        Some("structured-json-sse")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.model")
            .map(String::as_str),
        Some("glm-5.1")
    );
    assert!(!request.attributes.contains_key("url.path"));
    assert!(!request.attributes.contains_key("server.address"));
}

#[test]
fn structured_json_sse_model_name_messages_without_context_emits_no_llm_actions() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = concat!(
        "{\"messages\":[",
        "{\"role\":\"user\",\"content\":\"hello\"}",
        "],\"model_name\":\"catalog-v1\"}"
    );

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        response_segment_id(96),
        response_operation_id(96),
        response_sequence(96),
        http1_json_request_bytes("/api/ide/v2/llm_raw_chat", "api.enterprise.trae.cn", body),
    ));

    assert!(output.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmRequest
            && action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::LlmCall
    }));
}

#[test]
fn structured_json_sse_chunked_done_event_completes_response() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let first_body = concat!(
        "event: metadata\n",
        "data: {\"model\":\"glm-5.1\"}\n\n",
        "event: output\n",
        "data: {\"response\":\"你好\",\"reasoning_content\":null}\n\n",
    );
    let done_body = concat!("event: done\n", "data: {\"finish_reason\":\"stop\"}\n\n",);

    for (index, bytes) in [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(first_body),
        first_body.as_bytes().to_vec(),
        b"\r\n".to_vec(),
        http_chunk_prefix(done_body),
    ]
    .into_iter()
    .enumerate()
    {
        runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
    }

    let final_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(5),
        response_operation_id(5),
        response_sequence(5),
        done_body.as_bytes().to_vec(),
    ));

    let response = final_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("Trae done SSE event should complete llm.response");
    assert_eq!(response.status, SemanticActionStatus::Success);
    assert_eq!(response.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(
        response
            .attributes
            .get("llm.response.done")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.content_text")
            .map(String::as_str),
        Some("你好")
    );

    let stream = final_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::SseStream)
        .expect("Trae done SSE event should complete sse.stream");
    assert_eq!(stream.status, SemanticActionStatus::Success);
    assert_eq!(stream.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(
        stream.attributes.get("sse.done").map(String::as_str),
        Some("true")
    );
}

#[test]
fn structured_json_sse_non_llm_request_emits_no_llm_actions() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = r#"{"function":"chat"}"#;

    let output = runtime.observe_payload_segment(&outbound_http1_payload_segment_with_bytes(
        agent,
        STRUCTURED_NON_LLM_SEGMENT_ID,
        STRUCTURED_NON_LLM_OPERATION_ID,
        STRUCTURED_NON_LLM_SEQUENCE,
        http1_json_request_bytes(
            "/api/ide/v1/cli/get_config_list",
            "api.enterprise.trae.cn",
            body,
        ),
    ));

    assert!(output.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmRequest
            && action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::LlmCall
    }));
}

fn structured_json_sse_response_http_bytes() -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        STRUCTURED_JSON_SSE_RESPONSE.len(),
        STRUCTURED_JSON_SSE_RESPONSE
    )
    .into_bytes()
}

fn http1_json_request_bytes(path: &str, host: &str, body: &str) -> Vec<u8> {
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

fn structured_json_sse_relaxed_request_http_bytes() -> Vec<u8> {
    let body = structured_json_sse_request_body();
    http1_json_request_bytes("/v1/structured/stream", "llm.example.test", body)
}

fn structured_json_sse_request_body() -> &'static str {
    concat!(
        "{\"messages\":[",
        "{\"role\":\"system\",\"content\":\"system context redacted\"},",
        "{\"role\":\"user\",\"content\":\"hello\"}",
        "],\"model_name\":\"glm-5.1\",\"tools\":[]}"
    )
}

fn http2_request_bytes(stream_id: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = http2_frame(0x1, stream_id, b"\x82");
    bytes.extend(http2_frame(0x0, stream_id, body));
    bytes
}

fn http2_frame(frame_type: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let length = payload.len();
    let mut frame = Vec::with_capacity(9 + length);
    frame.push(((length >> 16) & 0xff) as u8);
    frame.push(((length >> 8) & 0xff) as u8);
    frame.push((length & 0xff) as u8);
    frame.push(frame_type);
    frame.push(0);
    frame.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[test]
fn llm_request_projects_canonical_content_blocks() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let output = runtime.observe_payload_segment(&llm_payload_segment(agent));
    let request = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("payload should project an llm.request action");

    assert_eq!(
        request
            .attributes
            .get("llm.request.content_state")
            .map(String::as_str),
        Some("canonical_blocks")
    );
    assert_eq!(
        request
            .attributes
            .get("llm.request.message_preview")
            .map(String::as_str),
        Some("hello")
    );
    assert!(!request.attributes.contains_key("llm.request.body_json"));
    assert_eq!(output.llm_request_contents.len(), 1);
    let content = &output.llm_request_contents[0];
    assert_eq!(content.manifest.action_id, request.action_id);
    assert_eq!(content.manifest.format_version, 1);
    assert!(!content.block_refs.is_empty());
    assert!(!content.blocks.is_empty());
}

#[test]
fn invalid_json_llm_request_is_shape_only() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let body = r#"{"model":"deepseek-chat","messages":"#;
    let bytes = format!(
        "POST /chat/completions HTTP/1.1\r\nHost: api.local\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let mut segment = llm_payload_segment(agent);
    let size = bytes.len() as u64;
    segment.bytes = bytes;
    segment.original_size = size;
    segment.captured_size = size;
    segment.operation_original_size = size;
    segment.operation_captured_size = size;

    let output = runtime.observe_payload_segment(&segment);
    let request = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("lossy LLM request should still project an action");

    assert_eq!(
        request
            .attributes
            .get("llm.request.content_state")
            .map(String::as_str),
        Some("shape")
    );
    assert!(!request.attributes.contains_key("llm.request.body_text"));
    assert!(output.llm_request_contents.is_empty());
}

#[test]
fn llm_response_updates_one_action_with_multiple_sse_segments() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let mut first_response = None;
    for (index, bytes) in [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(
            "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":null,\"reasoning_content\":\"hel\"}}]}\n\n",
        ),
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":null,\"reasoning_content\":\"hel\"}}]}\n\n"
            .as_bytes()
            .to_vec(),
        b"\r\n".to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
        let output = runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
        if first_response.is_none() {
            first_response = output
                .actions
                .into_iter()
                .find(|action| action.kind == SemanticActionKind::LlmResponse);
        }
    }

    runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        http_chunk_prefix(
            r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":null,"reasoning_content":"lo"}}]}

data: [DONE]

"#,
        ),
    ));
    let final_body_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(5),
        response_operation_id(5),
        response_sequence(5),
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":null,"reasoning_content":"lo"}}]}

data: [DONE]

"#
        .as_bytes()
        .to_vec(),
    ));
    let terminal_chunk_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(6),
        response_operation_id(6),
        response_sequence(6),
        b"\r\n0\r\n\r\n".to_vec(),
    ));
    let first_response = first_response
        .as_ref()
        .expect("first SSE segment should create an in-progress llm.response");
    let second_response = final_body_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second SSE segment should update the same llm.response");
    assert_eq!(second_response.action_id, first_response.action_id);
    let terminal_response = terminal_chunk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("chunk terminator should finalize the same llm.response");
    assert_eq!(terminal_response.action_id, first_response.action_id);
    assert_eq!(second_response.status, SemanticActionStatus::Success);
    assert_eq!(
        second_response.completeness,
        SemanticActionCompleteness::Complete
    );
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.model")
            .map(String::as_str),
        Some("deepseek-v4-flash")
    );
    assert!(second_response.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadAggregate
            && evidence.id == RESPONSE_FIRST_SEGMENT_ID.get()
            && evidence.role == "llm.response.payload"
    }));
    assert_eq!(
        second_response
            .evidence
            .iter()
            .filter(|evidence| evidence.role == "llm.response.payload")
            .count(),
        1
    );
    let expected_sequence_end = response_sequence(5).to_string();
    assert_eq!(
        second_response
            .attributes
            .get("payload.sequence_end")
            .map(String::as_str),
        Some(expected_sequence_end.as_str())
    );
}

#[test]
fn llm_response_stream_state_keeps_chunked_boundary_after_done() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first_body = concat!(
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":"first"}}]}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    let first_segments = [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(first_body),
        first_body.as_bytes().to_vec(),
        b"\r\n0\r\n\r\n".to_vec(),
    ];
    let mut first_response_id = None;
    for (index, bytes) in first_segments.into_iter().enumerate() {
        let output = runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
        if let Some(response) = output
            .actions
            .iter()
            .find(|action| action.kind == SemanticActionKind::LlmResponse)
        {
            first_response_id = Some(response.action_id.clone());
        }
    }
    let first_response_id = first_response_id.expect("first response should project");

    let second_body = concat!(
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"content":"second"}}]}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    let second_bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        second_body.len(),
        second_body,
    )
    .into_bytes();
    let second_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        second_bytes,
    ));
    let second_response = second_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second response should project independently");

    assert_ne!(second_response.action_id, first_response_id);
    assert!(!second_response.action_id.contains(":raw:llm.response"));
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.model")
            .map(String::as_str),
        Some("deepseek-v4-flash")
    );
}

#[test]
fn llm_response_suppresses_repeated_in_progress_boundaries_and_emits_semantic_updates() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    for (index, bytes) in [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec(),
        http_chunk_prefix(
            "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"hel\"}}]}\n\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        runtime.observe_payload_segment(&llm_response_payload_segment(
            agent.clone(),
            response_segment_id(index),
            response_operation_id(index),
            response_sequence(index),
            bytes,
        ));
    }

    let first = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(2),
        response_operation_id(2),
        response_sequence(2),
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"hel\"}}]}\n\n"
            .as_bytes()
            .to_vec(),
    ));
    let first_response = first
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first semantic SSE chunk should emit llm.response");
    let boundary = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(3),
        response_operation_id(3),
        response_sequence(3),
        b"\r\n".to_vec(),
    ));
    runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        http_chunk_prefix(
            r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"reasoning_content":"lo"}}]}

"#,
        ),
    ));
    let second = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(5),
        response_operation_id(5),
        response_sequence(5),
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"reasoning_content":"lo"}}]}

        "#
        .as_bytes()
        .to_vec(),
    ));

    assert!(
        boundary
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::LlmResponse)
    );
    let second_response = second
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("new semantic SSE content should update llm.response");
    assert_eq!(second_response.action_id, first_response.action_id);
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.reasoning_text")
            .map(String::as_str),
        Some("hello")
    );
}

#[test]
fn llm_response_stream_state_evicts_completed_messages() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first_body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"first"}}]}"#;
    let first_bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        first_body.len(),
        first_body
    )
    .into_bytes();
    let first_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(0),
        response_operation_id(0),
        response_sequence(0),
        first_bytes,
    ));
    let first_response = first_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first response should project an llm.response");

    let second_body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"second"}}]}"#;
    let second_bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        second_body.len(),
        second_body
    )
    .into_bytes();
    let second_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(1),
        response_operation_id(1),
        response_sequence(1),
        second_bytes,
    ));
    let second_response = second_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second response should project an llm.response");

    assert_ne!(second_response.action_id, first_response.action_id);
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.model")
            .map(String::as_str),
        Some("deepseek-chat")
    );
    assert!(second_response.evidence.iter().any(|evidence| {
        evidence.id == response_segment_id(1).get() && evidence.role == "llm.response.payload"
    }));
    assert!(second_response.evidence.iter().all(|evidence| {
        evidence.id != response_segment_id(0).get() || evidence.role != "llm.response.payload"
    }));
}

#[test]
fn raw_llm_responses_use_distinct_action_ids_after_evict() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(0),
        response_operation_id(0),
        response_sequence(0),
        concat!(
            r#"data: {"model":"deepseek-chat","choices":[{"delta":{"content":"first"}}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes()
        .to_vec(),
    ));
    let first_response = first
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first raw SSE response should project");

    let chunk_crlf = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(1),
        response_operation_id(1),
        response_sequence(1),
        b"\r\n".to_vec(),
    ));
    assert!(chunk_crlf.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::SseStream
            && action.kind != SemanticActionKind::SseEvent
    }));
    let zero_chunk = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(2),
        response_operation_id(2),
        response_sequence(2),
        b"0\r\n\r\n".to_vec(),
    ));
    assert!(zero_chunk.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmResponse
            && action.kind != SemanticActionKind::SseStream
            && action.kind != SemanticActionKind::SseEvent
    }));

    let second = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(3),
        response_operation_id(3),
        response_sequence(3),
        concat!(
            r#"data: {"model":"deepseek-chat","choices":[{"delta":{"content":"second"}}]}"#,
            "\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes()
        .to_vec(),
    ));
    let second_response = second
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second raw SSE response should project");

    assert_ne!(first_response.action_id, second_response.action_id);
    assert_eq!(
        second_response
            .attributes
            .get("llm.response.model")
            .map(String::as_str),
        Some("deepseek-chat")
    );
}

#[test]
fn llm_response_projects_sse_stream_summary_without_event_actions() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let sse = concat!(
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"bash","arguments":""}}]}}]}

"#,
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\""}}]}}]}

"#,
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"pwd\"}"}}]}}]}

"#,
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":\"answer\"}}]}\n\n",
        r#"data: {"model":"deepseek-v4-flash","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":7,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":5},"completion_tokens_details":{"reasoning_tokens":3},"prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":8}}

"#,
        "data: [DONE]\n\n"
    );
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        sse.len(),
        sse
    )
    .into_bytes();
    let output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        RESPONSE_FIRST_SEGMENT_ID,
        RESPONSE_FIRST_OPERATION_ID,
        RESPONSE_FIRST_SEQUENCE,
        bytes,
    ));
    let response = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("SSE payload should project an llm.response action");
    assert_eq!(
        response
            .attributes
            .get("llm.response.model")
            .map(String::as_str),
        Some("deepseek-v4-flash")
    );
    assert!(
        !response
            .attributes
            .contains_key("llm.response.payload_text")
    );
    assert!(!response.attributes.contains_key("http.response.body_text"));
    assert!(!response.attributes.contains_key("http.response.body_json"));
    assert!(!response.attributes.contains_key("llm.response.output_text"));
    assert_eq!(
        response
            .attributes
            .get("llm.response.provider_id")
            .map(String::as_str),
        Some("openai-compatible")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.content_text")
            .map(String::as_str),
        Some("answer")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.reasoning_text")
            .map(String::as_str),
        Some("thinking")
    );
    assert!(
        !response
            .attributes
            .contains_key("llm.response.sse_events_json")
    );
    let tool_calls_json = response
        .attributes
        .get("llm.response.tool_calls_json")
        .expect("SSE tool call deltas should be assembled on llm.response");
    let tool_calls = serde_json::from_str::<Value>(tool_calls_json)
        .expect("assembled llm.response.tool_calls_json should be valid JSON");
    assert_eq!(tool_calls[0]["id"], "call_1");
    assert_eq!(tool_calls[0]["type"], "function");
    assert_eq!(tool_calls[0]["function"]["name"], "bash");
    assert_eq!(
        tool_calls[0]["function"]["arguments"],
        r#"{"command":"pwd"}"#
    );
    assert_eq!(
        tool_calls[0]["function"]["arguments_json"]["command"],
        "pwd"
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.prompt_tokens")
            .map(String::as_str),
        Some("12")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.completion_tokens")
            .map(String::as_str),
        Some("7")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.total_tokens")
            .map(String::as_str),
        Some("19")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.cached_prompt_tokens")
            .map(String::as_str),
        Some("5")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.reasoning_tokens")
            .map(String::as_str),
        Some("3")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.prompt_cache_hit_tokens")
            .map(String::as_str),
        Some("4")
    );
    assert_eq!(
        response
            .attributes
            .get("llm.response.prompt_cache_miss_tokens")
            .map(String::as_str),
        Some("8")
    );

    let stream = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::SseStream)
        .expect("SSE payload should project an sse.stream action");
    assert_eq!(
        stream.attributes.get("sse.event_count").map(String::as_str),
        Some("7")
    );
    assert_eq!(
        stream
            .attributes
            .get("sse.content_delta_count")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        stream
            .attributes
            .get("sse.reasoning_delta_count")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        stream
            .attributes
            .get("sse.tool_delta_count")
            .map(String::as_str),
        Some("3")
    );
    assert_eq!(
        response
            .evidence
            .iter()
            .filter(|evidence| evidence.role == "llm.response.payload")
            .count(),
        1
    );
    assert!(response.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadAggregate
            && evidence.role == "llm.response.payload"
            && evidence.id == RESPONSE_FIRST_SEGMENT_ID.get()
    }));
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::SseEvent)
    );
    let events = output
        .actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::SseEvent)
        .collect::<Vec<_>>();
    assert!(events.is_empty());
    assert!(output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmResponseSseStream
            && link.parent_action_id == response.action_id
            && link.child_action_id == stream.action_id
    }));
}

#[test]
fn parsed_sse_storage_config_keeps_provider_events_on_sse_stream_action() {
    let mut semantic_retention = SemanticRetentionConfig::default();
    semantic_retention.content_owner = SemanticContentOwner::ConfiguredLayers;
    semantic_retention.l1_sse.event_content = SseEventContentRetention::Parsed;
    let mut runtime = LiveSemanticActionRuntime::new(
        AgentInvocationConfig {
            enabled: true,
            commands: vec!["xiaoo".to_string()],
        },
        semantic_retention,
        config_core::daemon::FileObservationConfig::default(),
    );
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let sse = concat!(
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
        r#"data: {"model":"deepseek-v4-flash","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"bash","arguments":"{\"command\":\"pwd\"}"}}]}}]}

"#,
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":\"answer\"}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
        sse.len(),
        sse
    )
    .into_bytes();
    let output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        RESPONSE_FIRST_SEGMENT_ID,
        RESPONSE_FIRST_OPERATION_ID,
        RESPONSE_FIRST_SEQUENCE,
        bytes,
    ));
    let response = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("SSE payload should project an llm.response action");
    assert!(
        !response
            .attributes
            .contains_key("llm.response.sse_events_json")
    );
    let stream = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::SseStream)
        .expect("SSE payload should project an sse.stream action");
    let events_json = stream
        .attributes
        .get("sse.events_json")
        .expect("parsed SSE storage should be retained when configured");
    let events =
        serde_json::from_str::<Value>(events_json).expect("sse.events_json should be valid JSON");

    assert_eq!(events.as_array().map(Vec::len), Some(4));
    assert_eq!(events[0]["reasoning_text"], "thinking");
    assert_eq!(events[1]["tool_calls"][0]["function"]["name"], "bash");
    assert_eq!(events[2]["content_text"], "answer");
    assert_eq!(events[3]["done"], true);
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::SseEvent)
    );
}

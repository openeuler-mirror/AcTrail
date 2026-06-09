use model_core::process::ProcessIdentity;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

use super::test_support::*;

#[test]
fn llm_response_links_to_request_and_http_response_message() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let request_output = runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("payload should project an llm.request action");

    let body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"project path"}}]}"#;
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let response_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        RESPONSE_FIRST_SEGMENT_ID,
        RESPONSE_FIRST_OPERATION_ID,
        RESPONSE_FIRST_SEQUENCE,
        bytes,
    ));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("payload should project an llm.response action");
    let exchange_link = response_output
        .links
        .iter()
        .find(|link| link.role == SemanticActionLinkRole::LlmRequestLlmResponse)
        .expect("llm.response should link to its preceding request");
    assert_eq!(exchange_link.parent_action_id, request.action_id);
    assert_eq!(exchange_link.child_action_id, response.action_id);

    let http_output = runtime.observe_event(&http_response_event(HTTP_RESPONSE_EVENT_ID, agent));
    let http_message = http_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("application event should project an http.message action");
    let http_link = http_output
        .links
        .iter()
        .find(|link| link.role == SemanticActionLinkRole::LlmResponseHttpMessage)
        .expect("inbound http.message should be linked under llm.response");
    assert_eq!(http_link.parent_action_id, response.action_id);
    assert_eq!(http_link.child_action_id, http_message.action_id);
}

#[test]
fn later_llm_request_does_not_link_to_previous_response() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));

    let body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"first"}}]}"#;
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let response_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        RESPONSE_FIRST_SEGMENT_ID,
        RESPONSE_FIRST_OPERATION_ID,
        RESPONSE_FIRST_SEQUENCE,
        bytes,
    ));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first response should project");

    let mut later_request = llm_payload_segment(agent);
    later_request.segment_id = response_segment_id(10);
    later_request.operation_id = response_operation_id(10);
    later_request.sequence = response_sequence(10);
    let later_output = runtime.observe_payload_segment(&later_request);

    assert!(later_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::LlmRequestLlmResponse
            || link.child_action_id != response.action_id
    }));
}

#[test]
fn llm_request_does_not_link_preceding_connect_tunnel_messages() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let connect_output =
        runtime.observe_event(&http_connect_event(HTTP_CONNECT_EVENT_ID, agent.clone()));
    let connect = connect_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("CONNECT request should project an http.message");
    let connect_response_output = runtime.observe_event(&http_connect_response_event(
        HTTP_CONNECT_RESPONSE_EVENT_ID,
        agent.clone(),
    ));
    let connect_response = connect_response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("CONNECT response should project an http.message");

    let request_output = runtime.observe_payload_segment(&llm_payload_segment(agent));
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("payload should project an llm.request action");

    for http_message in [connect, connect_response] {
        assert!(request_output.links.iter().all(|link| {
            link.role != SemanticActionLinkRole::LlmRequestHttpMessage
                || link.child_action_id != http_message.action_id
                || link.parent_action_id != request.action_id
        }));
    }
}

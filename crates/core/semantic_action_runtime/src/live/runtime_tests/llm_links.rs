use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::EventId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus,
};

#[path = "llm_links/fixtures.rs"]
mod fixtures;
#[path = "llm_links/http1.rs"]
mod http1;
#[path = "llm_links/http2.rs"]
mod http2;

use fixtures::*;

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
    let request_call = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("payload should project an llm.call action");
    let request_call_link = request_output
        .links
        .iter()
        .find(|link| link.role == SemanticActionLinkRole::LlmCallRequest)
        .expect("llm.call should link to its request");
    assert_eq!(request_call_link.parent_action_id, request_call.action_id);
    assert_eq!(request_call_link.child_action_id, request.action_id);

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
    let response_call = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("response should update the llm.call action");
    let exchange_link = response_output
        .links
        .iter()
        .find(|link| link.role == SemanticActionLinkRole::LlmCallResponse)
        .expect("llm.call should link to its response");
    assert_eq!(response_call.action_id, request_call.action_id);
    assert_eq!(exchange_link.parent_action_id, response_call.action_id);
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

    let later_call = later_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("later request should create its own llm.call");
    assert!(
        !later_call
            .attributes
            .contains_key("llm.call.response_action_id")
    );
    assert!(later_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::LlmRequestLlmResponse
            || link.child_action_id != response.action_id
    }));
    assert!(later_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::LlmCallResponse
            || link.child_action_id != response.action_id
    }));
}

#[test]
fn pending_outbound_request_blocks_response_from_pairing_with_previous_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let first_request_output = runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    let first_call = first_request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("first request should create an llm.call");

    let (request_head, request_tail) = split_request_segment(agent.clone());
    let pending_output = runtime.observe_payload_segment(&request_head);
    assert!(pending_output.actions.is_empty());

    let response_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(20),
        response_operation_id(20),
        response_sequence(20),
        json_response_bytes("pending request response"),
    ));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("response should project before pending request is complete");
    assert!(response_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::LlmCallResponse
            || link.parent_action_id != first_call.action_id
    }));

    let second_request_output = runtime.observe_payload_segment(&request_tail);
    let second_call = second_request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("completed pending request should create an llm.call");
    assert_ne!(second_call.action_id, first_call.action_id);
    assert_eq!(
        second_call
            .attributes
            .get("llm.call.response_action_id")
            .map(String::as_str),
        Some(response.action_id.as_str())
    );
    assert!(second_request_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallResponse
            && link.parent_action_id == second_call.action_id
            && link.child_action_id == response.action_id
    }));
}

#[test]
fn llm_call_pairs_response_by_observed_time_when_payload_sequences_are_direction_local() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let request_at = UNIX_EPOCH + Duration::from_millis(100);
    let response_at = UNIX_EPOCH + Duration::from_millis(250);

    let mut request_segment = llm_payload_segment(agent.clone());
    request_segment.sequence = 900;
    request_segment.observed_at = request_at;
    let request_output = runtime.observe_payload_segment(&request_segment);
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("request should project");
    let request_call = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("request should open an llm.call");

    let mut response_segment = llm_response_payload_segment(
        agent,
        response_segment_id(24),
        response_operation_id(24),
        1,
        json_response_bytes("direction local sequence"),
    );
    response_segment.observed_at = response_at;
    let response_output = runtime.observe_payload_segment(&response_segment);
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("response should project");
    let response_call = response_output
        .actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::LlmCall && action.action_id == request_call.action_id
        })
        .expect("response should close the request call by observed time");

    assert_eq!(response_call.status, SemanticActionStatus::Success);
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
    assert!(response_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallResponse
            && link.parent_action_id == response_call.action_id
            && link.child_action_id == response.action_id
    }));
}

#[test]
fn pending_outbound_request_between_by_observed_time_blocks_previous_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let first_request_at = UNIX_EPOCH + Duration::from_millis(100);
    let pending_request_at = UNIX_EPOCH + Duration::from_millis(200);
    let response_at = UNIX_EPOCH + Duration::from_millis(250);
    let pending_request_tail_at = UNIX_EPOCH + Duration::from_millis(300);

    let mut first_request = llm_payload_segment(agent.clone());
    first_request.sequence = 100;
    first_request.observed_at = first_request_at;
    let first_request_output = runtime.observe_payload_segment(&first_request);
    let first_call = first_request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("first request should create an llm.call");

    let (mut request_head, mut request_tail) = split_request_segment(agent.clone());
    request_head.sequence = 1;
    request_head.observed_at = pending_request_at;
    request_tail.sequence = 3;
    request_tail.observed_at = pending_request_tail_at;
    let pending_output = runtime.observe_payload_segment(&request_head);
    assert!(pending_output.actions.is_empty());

    let mut response_segment = llm_response_payload_segment(
        agent.clone(),
        response_segment_id(28),
        response_operation_id(28),
        2,
        json_response_bytes("second request response"),
    );
    response_segment.observed_at = response_at;
    let response_output = runtime.observe_payload_segment(&response_segment);
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("response should project before pending request is complete");
    assert!(response_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::LlmCallResponse
            || link.parent_action_id != first_call.action_id
    }));

    let second_request_output = runtime.observe_payload_segment(&request_tail);
    let second_call = second_request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("completed pending request should create an llm.call");
    assert_ne!(second_call.action_id, first_call.action_id);
    assert_eq!(
        second_call
            .attributes
            .get("llm.call.response_action_id")
            .map(String::as_str),
        Some(response.action_id.as_str())
    );
    assert!(second_request_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallResponse
            && link.parent_action_id == second_call.action_id
            && link.child_action_id == response.action_id
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

#[test]
fn raw_llm_response_links_only_to_current_http_response_message() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let stale_response = http_response_event_with(
        EventId::new(30),
        agent.clone(),
        response_segment_id(0),
        response_sequence(0),
        "404",
        "Not Found",
    );
    let stale_output = runtime.observe_event(&stale_response);
    let stale_http = stale_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("stale HTTP response should project");
    let current_response = http_response_event_with(
        EventId::new(31),
        agent.clone(),
        response_segment_id(1),
        response_sequence(1),
        "200",
        "OK",
    );
    let current_output = runtime.observe_event(&current_response);
    let current_http = current_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("current HTTP response should project");

    let first = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent.clone(),
        response_segment_id(2),
        response_operation_id(2),
        response_sequence(2),
        raw_sse_response_bytes("first"),
    ));
    let first_response = first
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("first raw SSE response should project");
    let response_http_links = first
        .links
        .iter()
        .filter(|link| link.role == SemanticActionLinkRole::LlmResponseHttpMessage)
        .collect::<Vec<_>>();
    assert_eq!(response_http_links.len(), 1);
    assert_eq!(
        response_http_links[0].parent_action_id,
        first_response.action_id
    );
    assert_eq!(
        response_http_links[0].child_action_id,
        current_http.action_id
    );
    assert_ne!(response_http_links[0].child_action_id, stale_http.action_id);

    let next_request = http_request_event_with(
        EventId::new(32),
        agent.clone(),
        response_segment_id(3),
        response_sequence(3),
    );
    runtime.observe_event(&next_request);
    let second = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(4),
        response_operation_id(4),
        response_sequence(4),
        raw_sse_response_bytes("second"),
    ));
    let second_response = second
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("second raw SSE response should project");
    assert!(second.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::LlmResponseHttpMessage
            || link.parent_action_id != second_response.action_id
    }));
}

#[test]
fn llm_call_ends_on_matching_http_error_response() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let request_output = runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    let request_call = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("LLM request should create an in-progress llm.call");
    assert_eq!(request_call.status, SemanticActionStatus::InProgress);

    let mut response_event = http_response_event_with(
        EventId::new(33),
        agent.clone(),
        response_segment_id(5),
        response_sequence(5),
        "404",
        "Not Found",
    );
    response_event.envelope.observed_at = UNIX_EPOCH + Duration::from_millis(250);
    let response_output = runtime.observe_event(&response_event);
    let failed_call = response_output
        .actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::LlmCall && action.action_id == request_call.action_id
        })
        .expect("HTTP error response should end the matching llm.call");

    assert_eq!(failed_call.status, SemanticActionStatus::Error);
    assert_eq!(
        failed_call.completeness,
        SemanticActionCompleteness::Complete
    );
    assert_eq!(
        failed_call.end_time,
        Some(UNIX_EPOCH + Duration::from_millis(250))
    );
    assert_eq!(
        failed_call
            .attributes
            .get("http.response.status_code")
            .map(String::as_str),
        Some("404")
    );
    assert!(
        !failed_call
            .attributes
            .contains_key("actrail.action.finalized_on_trace_close")
    );

    let finalized = runtime.finalize_trace(TRACE_ID, UNIX_EPOCH + Duration::from_secs(10));
    assert!(finalized.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmCall || action.action_id != request_call.action_id
    }));
}

#[test]
fn llm_call_ends_on_http_error_with_direction_local_payload_sequence() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let request_at = UNIX_EPOCH + Duration::from_millis(100);
    let response_at = UNIX_EPOCH + Duration::from_millis(250);
    let trace_close_at = UNIX_EPOCH + Duration::from_secs(10);
    let mut request_segment = llm_payload_segment(agent.clone());
    request_segment.sequence = RESPONSE_FIRST_SEQUENCE;
    request_segment.observed_at = request_at;
    let request_output = runtime.observe_payload_segment(&request_segment);
    let request_call = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("LLM request should create an in-progress llm.call");
    assert_eq!(request_call.status, SemanticActionStatus::InProgress);
    assert_eq!(request_call.start_time, request_at);
    assert_eq!(request_call.end_time, None);

    let mut response_event = http_response_event_with(
        EventId::new(34),
        agent,
        response_segment_id(6),
        RESPONSE_FIRST_SEQUENCE,
        "404",
        "Not Found",
    );
    response_event.envelope.observed_at = response_at;
    let response_output = runtime.observe_event(&response_event);
    let failed_call = response_output
        .actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::LlmCall && action.action_id == request_call.action_id
        })
        .expect("HTTP error response with direction-local sequence should end the llm.call");

    assert_eq!(failed_call.status, SemanticActionStatus::Error);
    assert_eq!(
        failed_call.completeness,
        SemanticActionCompleteness::Complete
    );
    assert_eq!(failed_call.start_time, request_at);
    assert_eq!(failed_call.end_time, Some(response_at));
    assert_eq!(
        failed_call
            .end_time
            .expect("HTTP 404 should close the call")
            .duration_since(failed_call.start_time)
            .expect("LLM call end time should not precede start time"),
        response_at
            .duration_since(request_at)
            .expect("test response time should follow request time")
    );
    assert_eq!(
        failed_call
            .attributes
            .get("http.response.status_code")
            .map(String::as_str),
        Some("404")
    );
    assert!(
        !failed_call
            .attributes
            .contains_key("actrail.action.finalized_on_trace_close")
    );

    let finalized = runtime.finalize_trace(TRACE_ID, trace_close_at);
    assert!(finalized.actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmCall || action.action_id != request_call.action_id
    }));
}

use model_core::process::ProcessIdentity;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

use super::super::test_support::*;
use super::fixtures::*;

#[test]
fn pending_outbound_request_on_other_http2_stream_does_not_block_response() {
    const FIRST_HTTP2_STREAM_ID: u32 = 1;
    const PENDING_HTTP2_STREAM_ID: u32 = 3;

    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let first_request_output = runtime.observe_payload_segment(&http2_llm_request_segment(
        agent.clone(),
        response_segment_id(30),
        response_operation_id(30),
        response_sequence(30),
        FIRST_HTTP2_STREAM_ID,
    ));
    let first_call = first_request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("first HTTP/2 request should create an llm.call");
    assert_eq!(
        first_call
            .attributes
            .get("http.request.stream_id")
            .map(String::as_str),
        Some("1")
    );

    let pending_output = runtime.observe_payload_segment(&http2_pending_request_head_segment(
        agent.clone(),
        response_segment_id(31),
        response_operation_id(31),
        response_sequence(31),
        PENDING_HTTP2_STREAM_ID,
    ));
    assert!(pending_output.actions.is_empty());

    let response_output = runtime.observe_payload_segment(&http2_llm_response_segment(
        agent,
        response_segment_id(32),
        response_operation_id(32),
        response_sequence(32),
        FIRST_HTTP2_STREAM_ID,
    ));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("HTTP/2 response should project");
    let updated_call = response_output
        .actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::LlmCall && action.action_id == first_call.action_id
        })
        .expect("response on the same HTTP/2 stream should update the first llm.call");
    assert_eq!(
        updated_call
            .attributes
            .get("llm.call.response_action_id")
            .map(String::as_str),
        Some(response.action_id.as_str())
    );
    assert!(response_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallResponse
            && link.parent_action_id == first_call.action_id
            && link.child_action_id == response.action_id
    }));
}

use model_core::process::ProcessIdentity;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

use super::super::test_support::*;
use super::fixtures::*;

#[test]
fn outbound_http1_request_resync_does_not_merge_body_prefix_with_next_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let prefix = outbound_payload_segment_with_bytes(
        agent.clone(),
        response_segment_id(40),
        response_operation_id(40),
        response_sequence(40),
        br#"{"leftover":"body"}"#.to_vec(),
    );
    let prefix_output = runtime.observe_payload_segment(&prefix);
    assert!(prefix_output.actions.is_empty());

    let mut request_template = llm_payload_segment(agent.clone());
    let request_bytes = std::mem::take(&mut request_template.bytes);
    let request_segment_id = response_segment_id(41);
    let request_output = runtime.observe_payload_segment(&outbound_payload_segment_with_bytes(
        agent.clone(),
        request_segment_id,
        response_operation_id(41),
        response_sequence(41),
        request_bytes,
    ));
    let request = request_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("valid request after a non-request prefix should project");
    let expected_segment_id = request_segment_id.get().to_string();
    assert_eq!(
        request
            .attributes
            .get("payload.aggregate.first_segment_id")
            .map(String::as_str),
        Some(expected_segment_id.as_str())
    );
    let expected_sequence = response_sequence(41).to_string();
    assert_eq!(
        request
            .attributes
            .get("payload.sequence_start")
            .map(String::as_str),
        Some(expected_sequence.as_str())
    );

    let response_output = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        response_segment_id(42),
        response_operation_id(42),
        response_sequence(42),
        json_response_bytes("ok"),
    ));
    let response = response_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmResponse)
        .expect("response should project");
    assert!(response_output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::LlmCallResponse
            && link.child_action_id == response.action_id
    }));
}

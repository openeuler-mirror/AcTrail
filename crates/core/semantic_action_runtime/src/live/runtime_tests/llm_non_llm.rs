use model_core::payload::{PayloadSegment, PayloadSegmentId};
use model_core::process::ProcessIdentity;
use semantic_action::SemanticActionKind;

use super::test_support::*;

const NON_LLM_REQUEST_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(900);
const NON_LLM_RESPONSE_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(901);
const LATER_LLM_REQUEST_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(1000);
const LATER_LLM_RESPONSE_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(1001);
const NON_LLM_REQUEST_OPERATION_ID: u64 = 900;
const NON_LLM_RESPONSE_OPERATION_ID: u64 = 901;
const LATER_LLM_REQUEST_OPERATION_ID: u64 = 1000;
const LATER_LLM_RESPONSE_OPERATION_ID: u64 = 1001;
const NON_LLM_REQUEST_SEQUENCE: u64 = 900;
const NON_LLM_RESPONSE_SEQUENCE: u64 = 901;
const LATER_LLM_REQUEST_SEQUENCE: u64 = 1000;
const LATER_LLM_RESPONSE_SEQUENCE: u64 = 1001;

#[test]
fn non_llm_http_request_does_not_block_later_llm_request() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first = runtime.observe_payload_segment(&non_llm_outbound_request(agent.clone()));
    assert_no_llm_actions(&first.actions);

    let mut later = llm_payload_segment(agent);
    later.segment_id = LATER_LLM_REQUEST_SEGMENT_ID;
    later.operation_id = LATER_LLM_REQUEST_OPERATION_ID;
    later.sequence = LATER_LLM_REQUEST_SEQUENCE;
    let second = runtime.observe_payload_segment(&later);

    assert!(
        second
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::LlmRequest)
    );
}

#[test]
fn non_llm_http_response_does_not_block_later_llm_response() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first = runtime.observe_payload_segment(&non_llm_inbound_response(agent.clone()));
    assert_no_llm_actions(&first.actions);

    let body = r#"{"model":"deepseek-chat","choices":[{"message":{"content":"ok"}}]}"#;
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let second = runtime.observe_payload_segment(&llm_response_payload_segment(
        agent,
        LATER_LLM_RESPONSE_SEGMENT_ID,
        LATER_LLM_RESPONSE_OPERATION_ID,
        LATER_LLM_RESPONSE_SEQUENCE,
        bytes,
    ));

    assert!(
        second
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::LlmResponse)
    );
}

fn non_llm_outbound_request(process: ProcessIdentity) -> PayloadSegment {
    let body = r#"{"not_model":"health"}"#;
    let bytes = format!(
        "POST /health HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let mut segment = llm_payload_segment(process);
    replace_segment_payload(
        &mut segment,
        NON_LLM_REQUEST_SEGMENT_ID,
        NON_LLM_REQUEST_OPERATION_ID,
        NON_LLM_REQUEST_SEQUENCE,
        bytes,
    );
    segment
}

fn non_llm_inbound_response(process: ProcessIdentity) -> PayloadSegment {
    let body = r#"{"ok":true}"#;
    let bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    llm_response_payload_segment(
        process,
        NON_LLM_RESPONSE_SEGMENT_ID,
        NON_LLM_RESPONSE_OPERATION_ID,
        NON_LLM_RESPONSE_SEQUENCE,
        bytes,
    )
}

fn replace_segment_payload(
    segment: &mut PayloadSegment,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    bytes: Vec<u8>,
) {
    let size = bytes.len() as u64;
    segment.segment_id = segment_id;
    segment.operation_id = operation_id;
    segment.sequence = sequence;
    segment.original_size = size;
    segment.captured_size = size;
    segment.operation_original_size = size;
    segment.operation_captured_size = size;
    segment.bytes = bytes;
}

fn assert_no_llm_actions(actions: &[semantic_action::SemanticAction]) {
    assert!(actions.iter().all(|action| {
        action.kind != SemanticActionKind::LlmRequest
            && action.kind != SemanticActionKind::LlmResponse
    }));
}

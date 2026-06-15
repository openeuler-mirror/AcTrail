use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus, SemanticEvidence,
    SemanticEvidenceKind,
};

use super::{ActionDisplayProjection, DisplayChild, ROOT_PARENT_ID};

#[test]
fn orphan_http_message_falls_back_to_same_process_command() {
    let process = ProcessIdentity::new(42, 100, 100);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "agent",
        process.clone(),
        1,
    );
    let http = action(
        "connect",
        SemanticActionKind::HttpMessage,
        "CONNECT api.example.test:443",
        process,
        2,
    );

    let projection = ActionDisplayProjection::new(vec![command.clone(), http.clone()], vec![]);

    assert_eq!(
        action_ids(&projection.children(ROOT_PARENT_ID)),
        vec![command.action_id]
    );
    assert_eq!(
        action_ids(&projection.children("command")),
        vec![http.action_id]
    );
    assert!(projection.children("command")[0].link.is_none());
}

#[test]
fn semantic_link_parent_wins_over_same_process_fallback() {
    let process = ProcessIdentity::new(43, 100, 100);
    let command = action(
        "command",
        SemanticActionKind::CommandInvocation,
        "agent",
        process.clone(),
        1,
    );
    let llm_request = action(
        "llm-request",
        SemanticActionKind::LlmRequest,
        "LLM request",
        process.clone(),
        2,
    );
    let http = action(
        "post",
        SemanticActionKind::HttpMessage,
        "POST /chat/completions",
        process,
        3,
    );
    let link = link(
        "llm-request",
        "post",
        SemanticActionLinkRole::LlmRequestHttpMessage,
    );

    let projection =
        ActionDisplayProjection::new(vec![command, llm_request, http.clone()], vec![link]);

    assert_eq!(
        action_ids(&projection.children("command")),
        vec!["llm-request".to_string()]
    );
    assert_eq!(
        action_ids(&projection.children("llm-request")),
        vec![http.action_id]
    );
    assert_eq!(
        projection.children("llm-request")[0]
            .link
            .as_ref()
            .map(|link| link.role),
        Some(SemanticActionLinkRole::LlmRequestHttpMessage)
    );
}

#[test]
fn stale_llm_response_http_links_are_not_display_parents() {
    let process = ProcessIdentity::new(44, 100, 100);
    let first_response = llm_response("response-1", process.clone(), 3, 42);
    let second_response = llm_response("response-2", process.clone(), 5, 44);
    let stale_http = http_response("http-404", process.clone(), 1, 40, "404");
    let current_http = http_response("http-200", process.clone(), 2, 41, "200");
    let next_request = http_request("next-request", process, 4, 43);
    let links = vec![
        link(
            "response-1",
            "http-404",
            SemanticActionLinkRole::LlmResponseHttpMessage,
        ),
        link(
            "response-1",
            "http-200",
            SemanticActionLinkRole::LlmResponseHttpMessage,
        ),
        link(
            "response-2",
            "http-200",
            SemanticActionLinkRole::LlmResponseHttpMessage,
        ),
    ];

    let projection = ActionDisplayProjection::new(
        vec![
            first_response,
            second_response,
            stale_http,
            current_http,
            next_request,
        ],
        links,
    );

    assert_eq!(
        action_ids(&projection.children("response-1")),
        vec!["http-200".to_string()]
    );
    assert!(projection.children("response-2").is_empty());
}

#[test]
fn finalized_llm_call_uses_matching_http_error_response_time() {
    let process = ProcessIdentity::new(45, 100, 100);
    let call = finalized_llm_call("call", "request", process.clone(), 1, 600_000);
    let request = llm_request("request", process.clone(), 1);
    let http_request = http_request("http-request", process.clone(), 2, 10);
    let http_response = http_response("http-404", process, 250, 11, "404");
    let request_link = link(
        "request",
        "http-request",
        SemanticActionLinkRole::LlmRequestHttpMessage,
    );

    let projection = ActionDisplayProjection::new(
        vec![call, request, http_request, http_response],
        vec![request_link],
    );
    let call = projection
        .actions
        .iter()
        .find(|action| action.action_id == "call")
        .expect("projected actions should keep the LLM call");

    assert_eq!(call.status, SemanticActionStatus::Error);
    assert_eq!(call.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(call.end_time, Some(UNIX_EPOCH + Duration::from_millis(251)));
    assert_eq!(
        call.attributes
            .get("http.response.status_code")
            .map(String::as_str),
        Some("404")
    );
    assert!(
        !call
            .attributes
            .contains_key("actrail.action.finalized_on_trace_close")
    );
}

fn action(
    id: &str,
    kind: SemanticActionKind,
    title: &str,
    process: ProcessIdentity,
    start_millis: u64,
) -> SemanticAction {
    SemanticAction {
        action_id: id.to_string(),
        trace_id: TraceId::new(1),
        kind,
        title: title.to_string(),
        start_time: UNIX_EPOCH + Duration::from_millis(start_millis),
        end_time: Some(UNIX_EPOCH + Duration::from_millis(start_millis + 1)),
        process,
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes: BTreeMap::new(),
        evidence: Vec::new(),
    }
}

fn llm_response(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmResponse,
        "LLM response",
        process,
        start_millis,
    );
    action
        .attributes
        .insert("payload.stream_key".to_string(), "stream-1".to_string());
    action.evidence.push(SemanticEvidence {
        kind: SemanticEvidenceKind::PayloadSegment,
        id: payload_segment_id,
        role: "llm.response.payload".to_string(),
    });
    action
}

fn finalized_llm_call(
    id: &str,
    request_id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    end_millis: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmCall,
        "LLM call",
        process,
        start_millis,
    );
    action.status = SemanticActionStatus::Error;
    action.completeness = SemanticActionCompleteness::Partial;
    action.end_time = Some(UNIX_EPOCH + Duration::from_millis(end_millis));
    action.attributes.insert(
        "actrail.action.finalized_on_trace_close".to_string(),
        "true".to_string(),
    );
    action.attributes.insert(
        "llm.call.request_action_id".to_string(),
        request_id.to_string(),
    );
    action
}

fn llm_request(id: &str, process: ProcessIdentity, start_millis: u64) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::LlmRequest,
        "LLM request",
        process,
        start_millis,
    );
    action
        .attributes
        .insert("payload.stream_key".to_string(), "stream-1".to_string());
    action
}

fn http_response(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
    status_code: &str,
) -> SemanticAction {
    let mut action = http_action(
        id,
        "HTTP response",
        process,
        start_millis,
        payload_segment_id,
    );
    action
        .attributes
        .insert("direction".to_string(), "inbound".to_string());
    action
        .attributes
        .insert("status_code".to_string(), status_code.to_string());
    action
}

fn http_request(
    id: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
) -> SemanticAction {
    let mut action = http_action(
        id,
        "HTTP request",
        process,
        start_millis,
        payload_segment_id,
    );
    action
        .attributes
        .insert("direction".to_string(), "outbound".to_string());
    action
}

fn http_action(
    id: &str,
    title: &str,
    process: ProcessIdentity,
    start_millis: u64,
    payload_segment_id: u64,
) -> SemanticAction {
    let mut action = action(
        id,
        SemanticActionKind::HttpMessage,
        title,
        process,
        start_millis,
    );
    action
        .attributes
        .insert("stream_key".to_string(), "stream-1".to_string());
    action.attributes.insert(
        "payload_segment_id".to_string(),
        payload_segment_id.to_string(),
    );
    action.attributes.insert(
        "payload_sequence".to_string(),
        payload_segment_id.to_string(),
    );
    action
}

fn link(parent: &str, child: &str, role: SemanticActionLinkRole) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: TraceId::new(1),
        parent_action_id: parent.to_string(),
        child_action_id: child.to_string(),
        role,
        confidence: SemanticActionLinkConfidence::Observed,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

fn action_ids(children: &[DisplayChild]) -> Vec<String> {
    children
        .iter()
        .map(|child| child.action.action_id.clone())
        .collect()
}

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
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

// Tests for the corresponding production module.
use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkOrigin, SemanticActionStatus,
};

use super::*;

#[test]
fn selected_parent_is_also_emitted_as_a_typed_span_link() {
    let action = SemanticAction {
        action_id: "child".to_string(),
        trace_id: TraceId::new(1),
        kind: SemanticActionKind::LlmToolResult,
        title: "tool result".to_string(),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: Some(SystemTime::UNIX_EPOCH),
        process: ProcessIdentity::new(7),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        attributes: BTreeMap::new(),
        evidence: Vec::new(),
    };
    let link = SemanticActionLink {
        trace_id: action.trace_id,
        parent_action_id: "llm-request".to_string(),
        child_action_id: action.action_id.clone(),
        role: SemanticActionLinkRole::LlmRequestToolResult,
        origin: SemanticActionLinkOrigin::Observed,
        valid: true,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    };
    let links = vec![link];
    let parent = parent_link(&action, &links);

    let emitted = support_links(&action, &links, parent).collect::<Vec<_>>();

    assert_eq!(
        parent.map(|link| link.role),
        Some(SemanticActionLinkRole::LlmRequestToolResult)
    );
    assert_eq!(emitted.len(), 1);
    assert_eq!(
        emitted[0].role,
        SemanticActionLinkRole::LlmRequestToolResult
    );
}

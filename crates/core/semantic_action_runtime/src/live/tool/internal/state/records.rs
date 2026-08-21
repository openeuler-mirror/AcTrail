//! Records owned exclusively by the tool interaction state aggregate.

use std::collections::BTreeSet;

use semantic_action::SemanticAction;

#[derive(Clone)]
pub(super) struct ToolCallRecord {
    pub(super) action: SemanticAction,
    pub(super) tool_call_id: Option<String>,
}

pub(super) struct AgentInvocationRecord {
    pub(super) action: SemanticAction,
    pub(super) prompt_message_hashes: BTreeSet<String>,
    pub(super) prompt_preview: Option<String>,
    pub(super) child_linked: bool,
}

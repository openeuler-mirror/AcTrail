//! Correlation policy for tool results and child agent requests.

use std::collections::BTreeSet;

use config_core::daemon::AgentInvocationConfig;
use semantic_action::{SemanticAction, attr_keys as attrs};
use serde_json::Value;

use super::prompt_fingerprint::PromptFingerprint;
use crate::live::tool::internal::declaration::DeclaredLlmToolCall;
use crate::live::tool::internal::state::{ToolCallCandidate, ToolInteractionState};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::live::tool) enum ToolResultBindingState {
    MissingId,
    Unmatched,
    Bound,
    Ambiguous,
}

impl ToolResultBindingState {
    pub(in crate::live::tool) fn as_str(self) -> &'static str {
        match self {
            Self::MissingId => "missing_id",
            Self::Unmatched => "unmatched",
            Self::Bound => "bound",
            Self::Ambiguous => "ambiguous",
        }
    }
}

pub(in crate::live::tool) struct ToolResultBinding {
    pub(in crate::live::tool) state: ToolResultBindingState,
    pub(in crate::live::tool) tool_call_action_id: Option<String>,
}

pub(in crate::live::tool) struct AgentInvocationCorrelator {
    enabled: bool,
    agent_tool_names: BTreeSet<String>,
}

impl AgentInvocationCorrelator {
    pub(in crate::live::tool) fn new(config: AgentInvocationConfig) -> Self {
        Self {
            enabled: config.enabled,
            agent_tool_names: config.tool_names.into_iter().collect(),
        }
    }

    pub(in crate::live::tool) fn projects_invocation(&self, tool_name: &str) -> bool {
        self.enabled && self.agent_tool_names.contains(tool_name)
    }

    pub(in crate::live::tool) fn invocation_prompt_key(
        &self,
        declared: &DeclaredLlmToolCall,
    ) -> (BTreeSet<String>, Option<String>) {
        declared
            .arguments
            .get("prompt")
            .and_then(Value::as_str)
            .map(PromptFingerprint::from_prompt)
            .unwrap_or_default()
            .into_parts()
    }

    pub(in crate::live::tool) fn bind_tool_result(
        &self,
        state: &ToolInteractionState,
        request: &SemanticAction,
        tool_call_id: Option<&str>,
    ) -> ToolResultBinding {
        let candidates = tool_call_id.map_or_else(Vec::new, |id| {
            state.tool_call_candidates(request.trace_id, id, &request.process)
        });
        let binding_state = match (tool_call_id, candidates.len()) {
            (None, _) => ToolResultBindingState::MissingId,
            (Some(_), 0) => ToolResultBindingState::Unmatched,
            (Some(_), 1) => ToolResultBindingState::Bound,
            (Some(_), _) => ToolResultBindingState::Ambiguous,
        };
        let tool_call_action_id = match candidates.as_slice() {
            [ToolCallCandidate { action_id }] => Some(action_id.clone()),
            _ => None,
        };
        ToolResultBinding {
            state: binding_state,
            tool_call_action_id,
        }
    }

    pub(in crate::live::tool) fn link_agent_child(
        &self,
        state: &mut ToolInteractionState,
        request: &SemanticAction,
        continuing_trajectory: bool,
    ) -> Option<SemanticAction> {
        if continuing_trajectory
            || request
                .attributes
                .contains_key(attrs::llm_request::BACKGROUND_KIND)
        {
            return None;
        }
        let request_hash = request
            .attributes
            .get(attrs::llm_request::LATEST_USER_MESSAGE_HASH);
        let request_preview = request.attributes.get(attrs::llm_request::MESSAGE_PREVIEW);
        let candidates = state.agent_child_candidates(
            request.trace_id,
            request_hash.map(String::as_str),
            request_preview.map(String::as_str),
        );
        let [action_id] = candidates.as_slice() else {
            return None;
        };
        state.link_agent_child(request.trace_id, action_id)
    }
}

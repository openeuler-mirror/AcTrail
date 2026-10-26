//! Tool-result correlation and agent-invocation classification.

use std::collections::BTreeSet;

use config_core::daemon::AgentInvocationConfig;
use semantic_action::SemanticAction;

use crate::live::tool::internal::state::{ToolCallCandidate, ToolInteractionState};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::live::tool) enum ToolResultBindingState {
    MissingId,
    Unmatched,
    Bound,
    Ambiguous,
}

impl ToolResultBindingState {
    pub(in crate::live::tool) fn semantic(self) -> semantic_action::SemanticToolResultBinding {
        use semantic_action::SemanticToolResultBinding as State;
        match self {
            Self::MissingId => State::MissingId,
            Self::Unmatched => State::Unmatched,
            Self::Bound => State::Bound,
            Self::Ambiguous => State::Ambiguous,
        }
    }
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
}

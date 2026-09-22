use std::collections::HashMap;

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
    SemanticActionStatus, attr_keys as attrs,
};
use storage_core::StorageBackend;

const RESULT_STATE: &str = "web.tool.result_state";

/// Read-only projection of declared tools and explicitly bound result observations.
pub(in crate::view) struct LlmToolDisplay;

#[derive(Clone, Copy, Default)]
enum ResultState {
    #[default]
    Unobserved,
    Returned,
    Error,
}

impl LlmToolDisplay {
    pub(in crate::view) fn normalize_loaded(
        actions: &mut [SemanticAction],
        links: &[SemanticActionLink],
    ) {
        let by_id = actions
            .iter()
            .filter(|action| {
                matches!(
                    action.kind,
                    SemanticActionKind::LlmToolCall | SemanticActionKind::LlmToolResult
                )
            })
            .map(|action| (action.action_id.as_str(), action))
            .collect::<HashMap<_, _>>();
        let mut outcomes = HashMap::<String, ResultState>::new();
        for link in links {
            let (Some(call), Some(result)) = (
                by_id.get(link.parent_action_id.as_str()),
                by_id.get(link.child_action_id.as_str()),
            ) else {
                continue;
            };
            if Self::bound_result(call, result, link) {
                let state = outcomes.entry(call.action_id.clone()).or_default();
                *state = Self::merge(*state, Self::result_state(result));
            }
        }
        for action in actions {
            let state = match action.kind {
                SemanticActionKind::LlmToolCall => {
                    outcomes.get(&action.action_id).copied().unwrap_or_default()
                }
                SemanticActionKind::LlmToolResult => Self::result_state(action),
                _ => continue,
            };
            Self::apply(action, state);
        }
    }

    pub(in crate::view) fn normalize_one(
        storage: &dyn StorageBackend,
        action: &mut SemanticAction,
    ) {
        if action.kind == SemanticActionKind::LlmToolResult {
            Self::apply(action, Self::result_state(action));
            return;
        }
        if action.kind != SemanticActionKind::LlmToolCall {
            return;
        }
        let mut state = ResultState::Unobserved;
        match storage.semantic_action_children(
            action.trace_id,
            &action.action_id,
            &[SemanticActionLinkRole::LlmToolCallResult.as_str()],
            &[],
        ) {
            Ok(rows) => {
                for row in rows {
                    if Self::bound_result(action, &row.action, &row.link) {
                        state = Self::merge(state, Self::result_state(&row.action));
                    }
                }
            }
            Err(error) => eprintln!(
                "actrailweb: tool result unavailable for {}: {}: {}",
                action.action_id, error.stage, error.message,
            ),
        }
        Self::apply(action, state);
    }

    fn bound_result(
        call: &SemanticAction,
        result: &SemanticAction,
        link: &SemanticActionLink,
    ) -> bool {
        let call_id = call.attributes.get(attrs::llm_tool_call::ID);
        link.valid
            && link.role == SemanticActionLinkRole::LlmToolCallResult
            && call.kind == SemanticActionKind::LlmToolCall
            && result.kind == SemanticActionKind::LlmToolResult
            && call.trace_id == result.trace_id
            && call.process == result.process
            && call_id.is_some()
            && call_id == result.attributes.get(attrs::llm_tool_result::ID)
            && result
                .attributes
                .get(attrs::llm_tool_result::BINDING_STATE)
                .is_some_and(|value| value == "bound")
            && ![call, result].iter().any(|action| {
                action
                    .attributes
                    .get(attrs::actrail::ACTION_VALID)
                    .is_some_and(|value| value == "false")
            })
    }

    fn result_state(result: &SemanticAction) -> ResultState {
        if result
            .attributes
            .get(attrs::llm_tool_result::IS_ERROR)
            .is_some_and(|value| value == "true")
        {
            ResultState::Error
        } else {
            // Missing protocol error flags were historically persisted as false/Success.
            ResultState::Returned
        }
    }

    fn merge(left: ResultState, right: ResultState) -> ResultState {
        if matches!(left, ResultState::Error) || matches!(right, ResultState::Error) {
            ResultState::Error
        } else {
            right
        }
    }

    fn apply(action: &mut SemanticAction, state: ResultState) {
        let (status, label) = match state {
            ResultState::Unobserved => (SemanticActionStatus::Unknown, "unobserved"),
            ResultState::Returned => (SemanticActionStatus::Unknown, "returned"),
            ResultState::Error => (SemanticActionStatus::Error, "error"),
        };
        action.status = status;
        action
            .attributes
            .insert(RESULT_STATE.to_string(), label.to_string());
    }
}

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionStatus, attr_keys as attrs,
};
use storage_core::StorageBackend;

const RESULT_STATE: &str = "web.tool.result_state";

/// Read-only projection of declared tools and the results that answer them.
///
/// A result answers the call with the same tool call id in the same process.
/// One instance serves one response, so a trace's results are read once.
#[derive(Default)]
pub(in crate::view) struct LlmToolDisplay {
    results: BTreeMap<TraceId, Vec<ObservedResult>>,
}

struct ObservedResult {
    process: ProcessIdentity,
    tool_call_id: String,
    start_time: SystemTime,
    state: ResultState,
}

#[derive(Clone, Copy, Default)]
enum ResultState {
    #[default]
    Unobserved,
    Returned,
    Error,
}

impl LlmToolDisplay {
    pub(in crate::view) fn normalize_one(
        &mut self,
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
        let state = self.answered_state(storage, action);
        Self::apply(action, state);
    }

    /// Ids can repeat within a trace; the first result after the call answers it.
    fn answered_state(
        &mut self,
        storage: &dyn StorageBackend,
        call: &SemanticAction,
    ) -> ResultState {
        let Some(call_id) = call.attributes.get(attrs::llm_tool_call::ID) else {
            return ResultState::Unobserved;
        };
        if Self::invalid(call) {
            return ResultState::Unobserved;
        }
        self.results
            .entry(call.trace_id)
            .or_insert_with(|| Self::observed_results(storage, call.trace_id))
            .iter()
            .filter(|result| {
                result.process == call.process
                    && result.tool_call_id == *call_id
                    && result.start_time >= call.start_time
            })
            .min_by_key(|result| result.start_time)
            .map_or(ResultState::Unobserved, |result| result.state)
    }

    fn observed_results(storage: &dyn StorageBackend, trace_id: TraceId) -> Vec<ObservedResult> {
        let results = storage
            .semantic_actions_matching_kinds_lite(
                trace_id,
                &[SemanticActionKind::LlmToolResult.as_str()],
            )
            .unwrap_or_else(|error| {
                eprintln!(
                    "actrailweb: tool results unavailable for trace {}: {}: {}",
                    trace_id.get(),
                    error.stage,
                    error.message,
                );
                Vec::new()
            });
        results
            .iter()
            .filter(|result| !Self::invalid(result))
            .filter_map(|result| {
                Some(ObservedResult {
                    process: result.process,
                    tool_call_id: result.attributes.get(attrs::llm_tool_call::ID)?.clone(),
                    start_time: result.start_time,
                    state: Self::result_state(result),
                })
            })
            .collect()
    }

    fn invalid(action: &SemanticAction) -> bool {
        action
            .attributes
            .get(attrs::actrail::ACTION_VALID)
            .is_some_and(|value| value == "false")
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

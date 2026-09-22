use std::collections::HashMap;
use std::time::SystemTime;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};
use storage_core::StorageBackend;

/// Derives a call's displayed outcome from its explicitly linked, completed response.
pub(in crate::view) struct LlmCallDisplay;

impl LlmCallDisplay {
    pub(in crate::view) fn normalize_loaded(actions: &mut [SemanticAction]) {
        let by_id = actions
            .iter()
            .map(|action| (action.action_id.as_str(), action))
            .collect::<HashMap<_, _>>();
        let outcomes = actions
            .iter()
            .map(|call| {
                let response_id = Self::response_id(call)?;
                Self::outcome(call, by_id.get(response_id)?)
            })
            .collect::<Vec<_>>();
        for (call, outcome) in actions.iter_mut().zip(outcomes) {
            Self::apply(call, outcome);
        }
    }

    pub(in crate::view) fn normalize_one(
        storage: &dyn StorageBackend,
        call: &mut SemanticAction,
    ) -> Result<(), String> {
        let Some(response_id) = Self::response_id(call) else {
            return Ok(());
        };
        let response = storage
            .semantic_action_by_id(call.trace_id, response_id)
            .map_err(|error| {
                format!(
                    "read model response failed: {}: {}",
                    error.stage, error.message
                )
            })?;
        let outcome = response
            .as_ref()
            .and_then(|response| Self::outcome(call, response));
        Self::apply(call, outcome);
        Ok(())
    }

    fn response_id(call: &SemanticAction) -> Option<&str> {
        (call.kind == SemanticActionKind::LlmCall && call.status == SemanticActionStatus::Unknown)
            .then(|| call.attributes.get(attrs::llm_call::RESPONSE_ACTION_ID))
            .flatten()
            .map(String::as_str)
    }

    fn outcome(
        call: &SemanticAction,
        response: &SemanticAction,
    ) -> Option<(SemanticActionStatus, SystemTime)> {
        if response.kind != SemanticActionKind::LlmResponse
            || response.trace_id != call.trace_id
            || response.process != call.process
            || response.completeness != SemanticActionCompleteness::Complete
            || response
                .attributes
                .get(attrs::actrail::ACTION_VALID)
                .is_some_and(|value| value == "false")
            || !matches!(
                response.status,
                SemanticActionStatus::Success | SemanticActionStatus::Error
            )
        {
            return None;
        }
        let end_time = response.end_time?;
        (end_time >= call.start_time).then_some((response.status, end_time))
    }

    fn apply(call: &mut SemanticAction, outcome: Option<(SemanticActionStatus, SystemTime)>) {
        if let Some((status, end_time)) = outcome {
            call.status = status;
            call.end_time = Some(end_time);
        }
    }
}

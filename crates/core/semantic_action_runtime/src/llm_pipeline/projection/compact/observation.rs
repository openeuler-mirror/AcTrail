use std::sync::Arc;
use std::time::SystemTime;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, SemanticEvidence,
    attr_keys as attrs,
};

use crate::llm_pipeline::provider::RequestHistorySource;

pub(super) use crate::llm_pipeline::provider::RequestHistoryLimit as CompactLimitExceeded;

pub(in crate::llm_pipeline) struct LlmRequestObservation {
    pub(in crate::llm_pipeline) action_id: String,
    pub(in crate::llm_pipeline) history: Arc<RequestHistorySource>,
    pub(in crate::llm_pipeline) context: crate::llm_pipeline::provider::ParsedRequestContext,
    /// Byte length of the entire decoded request JSON input used to parse body.
    pub(in crate::llm_pipeline) history_source_bytes: usize,
}

pub(in crate::llm_pipeline) struct LlmResponseObservation {
    pub(in crate::llm_pipeline) action_id: String,
    pub(in crate::llm_pipeline) content_text: Option<String>,
    pub(in crate::llm_pipeline) provider_failed: bool,
    /// Positive evidence from the parsed response; absent tool data stays unknown.
    pub(in crate::llm_pipeline) has_tool_calls: bool,
}

/// Terminal response facts owned by the current binding batch.
pub(in crate::llm_pipeline) struct CompletedResponse {
    pub(super) action_id: String,
    pub(super) start_time: SystemTime,
    pub(super) end_time: Option<SystemTime>,
    pub(super) valid_response: bool,
    pub(super) evidence: Vec<SemanticEvidence>,
    pub(super) content_text: Option<String>,
    pub(super) has_tool_calls: bool,
}

impl CompletedResponse {
    pub(in crate::llm_pipeline) fn from_terminal_action(
        action: &SemanticAction,
        observation: Option<LlmResponseObservation>,
    ) -> Option<Self> {
        if action.status == SemanticActionStatus::InProgress {
            return None;
        }
        let valid_response = action.status == SemanticActionStatus::Success
            && action.completeness == SemanticActionCompleteness::Complete
            && action.end_time.is_some()
            && observation
                .as_ref()
                .is_none_or(|value| value.action_id == action.action_id && !value.provider_failed)
            && action
                .attributes
                .get(attrs::http_response::STATUS_CODE)
                .is_none_or(|status| status.parse::<u16>().is_ok_and(|status| status < 400));
        let (action_id, content_text, has_tool_calls) = match observation {
            Some(value) if value.action_id == action.action_id => {
                (value.action_id, value.content_text, value.has_tool_calls)
            }
            _ => (action.action_id.clone(), None, false),
        };
        Some(Self {
            action_id,
            start_time: action.start_time,
            end_time: action.end_time,
            valid_response,
            evidence: if valid_response {
                action.evidence.clone()
            } else {
                Vec::new()
            },
            content_text,
            has_tool_calls,
        })
    }
}

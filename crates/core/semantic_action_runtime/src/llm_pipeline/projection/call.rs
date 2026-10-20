//! LLM request/response to call correlation.

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};
use std::collections::BTreeMap;

use crate::live::llm_call_action_id_from_request_action_id;

pub(in crate::llm_pipeline) fn llm_call_from_request_response(
    request: &SemanticAction,
    response_id: Option<&str>,
) -> SemanticAction {
    let mut attributes = BTreeMap::new();
    attributes.insert(
        attrs::llm_call::REQUEST_ACTION_ID.to_string(),
        request.action_id.clone(),
    );
    if let Some(response_id) = response_id {
        attributes.insert(
            attrs::llm_call::RESPONSE_ACTION_ID.to_string(),
            response_id.to_owned(),
        );
    }
    let completeness = if response_id.is_some() {
        SemanticActionCompleteness::Inferred
    } else {
        SemanticActionCompleteness::Partial
    };

    SemanticAction {
        action_id: llm_call_action_id_from_request_action_id(&request.action_id),
        trace_id: request.trace_id,
        kind: SemanticActionKind::LlmCall,
        title: "LLM call".to_string(),
        start_time: request.start_time,
        end_time: None,
        process: request.process.clone(),
        status: SemanticActionStatus::Unknown,
        completeness,
        attributes,
        evidence: Vec::new(),
    }
}

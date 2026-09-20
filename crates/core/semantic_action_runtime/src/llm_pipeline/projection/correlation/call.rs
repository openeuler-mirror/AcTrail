//! LLM request/response to call correlation.

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};
use std::collections::BTreeMap;

use crate::live::llm_call_action_id_from_request_action_id;

pub(in crate::llm_pipeline) fn llm_call_from_request_response(
    request: &SemanticAction,
    response: Option<&SemanticAction>,
) -> SemanticAction {
    let mut attributes = BTreeMap::new();
    attributes.insert(
        attrs::llm_call::REQUEST_ACTION_ID.to_string(),
        request.action_id.clone(),
    );
    if let Some(response) = response {
        attributes.insert(
            attrs::llm_call::RESPONSE_ACTION_ID.to_string(),
            response.action_id.clone(),
        );
    }
    let completeness = if response.is_some() {
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

pub(in crate::llm_pipeline) fn payload_sequence_start(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(attrs::payload::SEQUENCE_START)
        .or_else(|| action.attributes.get(attrs::payload::SEQUENCE))
        .and_then(|value| value.parse().ok())
}

pub(in crate::llm_pipeline) fn payload_sequence_end(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(attrs::payload::SEQUENCE_END)
        .or_else(|| action.attributes.get(attrs::payload::SEQUENCE))
        .and_then(|value| value.parse().ok())
}

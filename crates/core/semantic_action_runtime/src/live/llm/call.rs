use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::live::actions::{append_missing_evidence, llm_call_action_id_from_request_action_id};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingLlmRequestMarker {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
    pub(super) stream_key: String,
    pub(super) http_stream_id: Option<String>,
    pub(super) start_time: SystemTime,
    pub(super) sequence_start: u64,
}

pub(super) fn llm_call_from_request_response(
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
    if let Some(model) = request
        .attributes
        .get(attrs::llm_request::MODEL)
        .or_else(|| response.and_then(|action| action.attributes.get(attrs::llm_response::MODEL)))
    {
        attributes.insert(attrs::llm_call::MODEL.to_string(), model.clone());
    }
    copy_attr(request, &mut attributes, attrs::payload::STREAM_KEY);
    copy_attr(request, &mut attributes, attrs::payload::OPERATION_ID);
    copy_attr(request, &mut attributes, attrs::http_request::STREAM_ID);
    if let Some(response) = response {
        copy_attr(response, &mut attributes, attrs::http_response::STATUS_CODE);
        copy_attr(response, &mut attributes, attrs::http_response::REASON);
    }

    let mut evidence = request.evidence.clone();
    if let Some(response) = response {
        append_missing_evidence(&mut evidence, &response.evidence);
    }
    let status = response
        .map(|action| action.status)
        .unwrap_or(SemanticActionStatus::InProgress);
    let completeness = response
        .map(|action| merge_llm_call_completeness(request.completeness, action.completeness))
        .unwrap_or(SemanticActionCompleteness::Partial);
    let title = attributes
        .get(attrs::llm_call::MODEL)
        .map(|model| format!("LLM call {model}"))
        .unwrap_or_else(|| "LLM call".to_string());

    SemanticAction {
        action_id: llm_call_action_id_from_request_action_id(&request.action_id),
        trace_id: request.trace_id,
        kind: SemanticActionKind::LlmCall,
        title,
        start_time: request.start_time,
        end_time: response.and_then(|action| action.end_time),
        process: request.process.clone(),
        status,
        completeness,
        confidence_millis: None,
        attributes,
        evidence,
    }
}

fn copy_attr(
    action: &SemanticAction,
    attributes: &mut BTreeMap<String, String>,
    key: &'static str,
) {
    if let Some(value) = action.attributes.get(key) {
        attributes.insert(key.to_string(), value.clone());
    }
}

fn merge_llm_call_completeness(
    request: SemanticActionCompleteness,
    response: SemanticActionCompleteness,
) -> SemanticActionCompleteness {
    match (request, response) {
        (SemanticActionCompleteness::Complete, SemanticActionCompleteness::Complete) => {
            SemanticActionCompleteness::Complete
        }
        (SemanticActionCompleteness::Inferred, SemanticActionCompleteness::Inferred) => {
            SemanticActionCompleteness::Inferred
        }
        _ => SemanticActionCompleteness::Partial,
    }
}

pub(super) fn payload_sequence_start(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(attrs::payload::SEQUENCE_START)
        .or_else(|| action.attributes.get(attrs::payload::SEQUENCE))
        .and_then(|value| value.parse().ok())
}

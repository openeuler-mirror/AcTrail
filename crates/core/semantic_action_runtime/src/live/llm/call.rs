use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::live::actions::{append_missing_evidence, llm_call_action_id_from_request_action_id};

pub(super) fn llm_call_actions_for(
    changed: &[SemanticAction],
    emitted_actions: &[&SemanticAction],
) -> Vec<SemanticAction> {
    changed
        .iter()
        .filter_map(|action| match action.kind {
            SemanticActionKind::LlmRequest => Some(llm_call_from_request_response(
                action,
                latest_response_for_request(action, emitted_actions).as_ref(),
            )),
            SemanticActionKind::LlmResponse => latest_request_for_response(action, emitted_actions)
                .map(|request| llm_call_from_request_response(&request, Some(action))),
            _ => None,
        })
        .collect()
}

fn latest_request_for_response(
    response: &SemanticAction,
    emitted_actions: &[&SemanticAction],
) -> Option<SemanticAction> {
    emitted_actions
        .iter()
        .copied()
        .filter(|request| llm_actions_share_stream(request, response))
        .filter(|request| request.start_time <= response.start_time)
        .max_by_key(|request| (request.start_time, request.action_id.clone()))
        .cloned()
}

fn latest_response_for_request(
    request: &SemanticAction,
    emitted_actions: &[&SemanticAction],
) -> Option<SemanticAction> {
    emitted_actions
        .iter()
        .copied()
        .filter(|response| llm_actions_share_stream(request, response))
        .filter(|response| request.start_time <= response.start_time)
        .max_by_key(|response| (response.start_time, response.action_id.clone()))
        .cloned()
}

fn llm_call_from_request_response(
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

fn llm_actions_share_stream(request: &SemanticAction, response: &SemanticAction) -> bool {
    request.kind == SemanticActionKind::LlmRequest
        && response.kind == SemanticActionKind::LlmResponse
        && request.trace_id == response.trace_id
        && request.process == response.process
        && request.attributes.get(attrs::payload::STREAM_KEY)
            == response.attributes.get(attrs::payload::STREAM_KEY)
        && http_stream_ids_are_compatible(request, response)
}

fn http_stream_ids_are_compatible(request: &SemanticAction, response: &SemanticAction) -> bool {
    match (
        request.attributes.get(attrs::http_request::STREAM_ID),
        response.attributes.get(attrs::http_response::STREAM_ID),
    ) {
        (Some(request_stream_id), Some(response_stream_id)) => {
            request_stream_id == response_stream_id
        }
        (None, None) => true,
        _ => false,
    }
}

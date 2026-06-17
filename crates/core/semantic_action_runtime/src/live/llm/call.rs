use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::live::actions::{append_missing_evidence, llm_call_action_id_from_request_action_id};

pub(super) fn llm_call_actions_for(
    changed: &[SemanticAction],
    emitted_actions: &[&SemanticAction],
    pending_requests: &[PendingLlmRequestMarker],
) -> Vec<SemanticAction> {
    changed
        .iter()
        .filter_map(|action| match action.kind {
            SemanticActionKind::LlmRequest => Some(llm_call_from_request_response(
                action,
                latest_response_for_request(action, emitted_actions, pending_requests).as_ref(),
            )),
            SemanticActionKind::LlmResponse => {
                latest_request_for_response(action, emitted_actions, pending_requests)
                    .map(|request| llm_call_from_request_response(&request, Some(action)))
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingLlmRequestMarker {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
    pub(super) stream_key: String,
    pub(super) http_stream_id: Option<String>,
    pub(super) sequence_start: u64,
}

fn latest_request_for_response(
    response: &SemanticAction,
    emitted_actions: &[&SemanticAction],
    pending_requests: &[PendingLlmRequestMarker],
) -> Option<SemanticAction> {
    let response_marker = payload_sequence_start(response)?;
    emitted_actions
        .iter()
        .copied()
        .filter(|request| llm_actions_share_stream(request, response))
        .filter(|request| {
            payload_sequence_start(request).is_some_and(|request_marker| {
                request_marker <= response_marker
                    && !request_between(request, response_marker, emitted_actions, pending_requests)
            })
        })
        .max_by_key(|request| {
            (
                payload_sequence_start(request).unwrap_or_default(),
                request.action_id.clone(),
            )
        })
        .cloned()
}

fn latest_response_for_request(
    request: &SemanticAction,
    emitted_actions: &[&SemanticAction],
    pending_requests: &[PendingLlmRequestMarker],
) -> Option<SemanticAction> {
    let request_marker = payload_sequence_start(request)?;
    let next_request_marker = next_request_marker(request, emitted_actions, pending_requests);
    emitted_actions
        .iter()
        .copied()
        .filter(|response| llm_actions_share_stream(request, response))
        .filter(|response| {
            payload_sequence_start(response).is_some_and(|response_marker| {
                request_marker <= response_marker
                    && next_request_marker.is_none_or(|next| response_marker < next)
            })
        })
        .max_by_key(|response| {
            (
                payload_sequence_start(response).unwrap_or_default(),
                response.action_id.clone(),
            )
        })
        .cloned()
}

fn request_between(
    request: &SemanticAction,
    response_marker: u64,
    emitted_actions: &[&SemanticAction],
    pending_requests: &[PendingLlmRequestMarker],
) -> bool {
    let Some(request_marker) = payload_sequence_start(request) else {
        return true;
    };
    emitted_actions.iter().copied().any(|candidate| {
        candidate.action_id != request.action_id
            && request_like_actions_share_stream(request, candidate)
            && payload_sequence_start(candidate)
                .is_some_and(|marker| request_marker < marker && marker <= response_marker)
    }) || pending_requests.iter().any(|pending| {
        pending.matches_request(request)
            && request_marker < pending.sequence_start
            && pending.sequence_start <= response_marker
    })
}

fn next_request_marker(
    request: &SemanticAction,
    emitted_actions: &[&SemanticAction],
    pending_requests: &[PendingLlmRequestMarker],
) -> Option<u64> {
    let request_marker = payload_sequence_start(request)?;
    let emitted = emitted_actions
        .iter()
        .copied()
        .filter(|candidate| candidate.action_id != request.action_id)
        .filter(|candidate| request_like_actions_share_stream(request, candidate))
        .filter_map(payload_sequence_start)
        .filter(|marker| *marker > request_marker);
    let pending = pending_requests
        .iter()
        .filter(|pending| pending.matches_request(request))
        .map(|pending| pending.sequence_start)
        .filter(|marker| *marker > request_marker);
    emitted.chain(pending).min()
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

fn request_like_actions_share_stream(left: &SemanticAction, right: &SemanticAction) -> bool {
    left.kind == SemanticActionKind::LlmRequest
        && right.kind == SemanticActionKind::LlmRequest
        && left.trace_id == right.trace_id
        && left.process == right.process
        && left.attributes.get(attrs::payload::STREAM_KEY)
            == right.attributes.get(attrs::payload::STREAM_KEY)
        && left.attributes.get(attrs::http_request::STREAM_ID)
            == right.attributes.get(attrs::http_request::STREAM_ID)
}

fn payload_sequence_start(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(attrs::payload::SEQUENCE_START)
        .or_else(|| action.attributes.get(attrs::payload::SEQUENCE))
        .and_then(|value| value.parse().ok())
}

impl PendingLlmRequestMarker {
    fn matches_request(&self, request: &SemanticAction) -> bool {
        request.kind == SemanticActionKind::LlmRequest
            && self.trace_id == request.trace_id
            && self.process == request.process
            && request
                .attributes
                .get(attrs::payload::STREAM_KEY)
                .is_some_and(|stream_key| stream_key == &self.stream_key)
            && request.attributes.get(attrs::http_request::STREAM_ID)
                == self.http_stream_id.as_ref()
    }
}

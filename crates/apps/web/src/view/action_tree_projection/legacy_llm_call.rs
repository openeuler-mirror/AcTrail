use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkRole, SemanticActionStatus,
};

const DIRECTION_ATTR: &str = "direction";
const DIRECTION_INBOUND: &str = "inbound";
const DIRECTION_OUTBOUND: &str = "outbound";
const FINALIZED_ON_TRACE_CLOSE_ATTR: &str = "actrail.action.finalized_on_trace_close";
const HTTP_RESPONSE_ACTION_ID_ATTR: &str = "llm.call.http_response_action_id";
const HTTP_RESPONSE_STATUS_CODE_ATTR: &str = "http.response.status_code";
const LLM_CALL_REQUEST_ACTION_ID_ATTR: &str = "llm.call.request_action_id";
const LLM_CALL_RESPONSE_ACTION_ID_ATTR: &str = "llm.call.response_action_id";
const HTTP_REQUEST_STREAM_ID_ATTR: &str = "http.request.stream_id";
const PAYLOAD_SEQUENCE_ATTR: &str = "payload.sequence";
const PAYLOAD_STREAM_KEY_ATTR: &str = "payload.stream_key";
const STATUS_CODE_ATTR: &str = "status_code";
const STREAM_KEY_ATTR: &str = "stream_key";
const HTTP_MESSAGE_STREAM_ID_ATTR: &str = "stream_id";
const HTTP_CLIENT_ERROR_MIN: u16 = 400;
const HTTP_SERVER_ERROR_MAX: u16 = 599;

pub(super) fn normalize_finalized_http_error_calls(
    actions: Vec<SemanticAction>,
    links: &[SemanticActionLink],
) -> Vec<SemanticAction> {
    let action_by_id = actions
        .iter()
        .map(|action| (action.action_id.clone(), action.clone()))
        .collect::<BTreeMap<_, _>>();
    let request_http_messages = request_http_messages_by_request(links);
    actions
        .into_iter()
        .map(|action| normalize_call(action, &action_by_id, &request_http_messages))
        .collect()
}

fn normalize_call(
    action: SemanticAction,
    action_by_id: &BTreeMap<String, SemanticAction>,
    request_http_messages: &BTreeMap<String, String>,
) -> SemanticAction {
    if !legacy_finalized_call_candidate(&action) {
        return action;
    }
    let Some(request_id) = action.attributes.get(LLM_CALL_REQUEST_ACTION_ID_ATTR) else {
        return action;
    };
    let Some(request) = action_by_id.get(request_id) else {
        return action;
    };
    let Some(request_marker) = request_marker(request, action_by_id, request_http_messages) else {
        return action;
    };
    let Some(response) = matching_http_error_response(request, request_marker, action_by_id) else {
        return action;
    };
    let mut normalized = action;
    normalized.status = SemanticActionStatus::Error;
    normalized.completeness = SemanticActionCompleteness::Complete;
    normalized.end_time = Some(response.end_time.unwrap_or(response.start_time));
    normalized.attributes.remove(FINALIZED_ON_TRACE_CLOSE_ATTR);
    normalized.attributes.insert(
        HTTP_RESPONSE_ACTION_ID_ATTR.to_string(),
        response.action_id.clone(),
    );
    if let Some(status_code) = response.attributes.get(STATUS_CODE_ATTR) {
        normalized.attributes.insert(
            HTTP_RESPONSE_STATUS_CODE_ATTR.to_string(),
            status_code.clone(),
        );
    }
    if let Some(reason) = response.attributes.get("reason") {
        normalized
            .attributes
            .insert("http.response.reason".to_string(), reason.clone());
    }
    normalized
}

fn request_http_messages_by_request(links: &[SemanticActionLink]) -> BTreeMap<String, String> {
    links
        .iter()
        .filter(|link| link.role == SemanticActionLinkRole::LlmRequestHttpMessage)
        .map(|link| (link.parent_action_id.clone(), link.child_action_id.clone()))
        .collect()
}

fn legacy_finalized_call_candidate(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::LlmCall
        && action.status == SemanticActionStatus::Error
        && action
            .attributes
            .get(FINALIZED_ON_TRACE_CLOSE_ATTR)
            .is_some_and(|value| value == "true")
        && !action
            .attributes
            .contains_key(LLM_CALL_RESPONSE_ACTION_ID_ATTR)
}

fn request_marker(
    request: &SemanticAction,
    action_by_id: &BTreeMap<String, SemanticAction>,
    request_http_messages: &BTreeMap<String, String>,
) -> Option<u64> {
    payload_sequence(request).or_else(|| {
        let http_id = request_http_messages.get(&request.action_id)?;
        let http_message = action_by_id.get(http_id)?;
        http_payload_sequence(http_message)
    })
}

fn matching_http_error_response<'a>(
    request: &SemanticAction,
    request_marker: u64,
    action_by_id: &'a BTreeMap<String, SemanticAction>,
) -> Option<&'a SemanticAction> {
    let next_request_marker = next_http_request_marker(request, request_marker, action_by_id);
    action_by_id
        .values()
        .filter(|candidate| http_error_response_matches_request(request, candidate))
        .filter_map(|candidate| Some((http_payload_sequence(candidate)?, candidate)))
        .filter(|(marker, _)| *marker > request_marker)
        .filter(|(marker, _)| next_request_marker.is_none_or(|next| *marker < next))
        .min_by(|left, right| {
            (left.0, left.1.action_id.as_str()).cmp(&(right.0, right.1.action_id.as_str()))
        })
        .map(|(_, candidate)| candidate)
}

fn next_http_request_marker(
    request: &SemanticAction,
    request_marker: u64,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> Option<u64> {
    action_by_id
        .values()
        .filter(|candidate| {
            candidate.kind == SemanticActionKind::HttpMessage
                && candidate.attributes.get(DIRECTION_ATTR).map(String::as_str)
                    == Some(DIRECTION_OUTBOUND)
                && same_trace_process_stream(request, candidate)
        })
        .filter_map(http_payload_sequence)
        .filter(|marker| *marker > request_marker)
        .min()
}

fn http_error_response_matches_request(
    request: &SemanticAction,
    http_response: &SemanticAction,
) -> bool {
    http_response.kind == SemanticActionKind::HttpMessage
        && http_response
            .attributes
            .get(DIRECTION_ATTR)
            .map(String::as_str)
            == Some(DIRECTION_INBOUND)
        && same_trace_process_stream(request, http_response)
        && http_response
            .attributes
            .get(STATUS_CODE_ATTR)
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(|status| (HTTP_CLIENT_ERROR_MIN..=HTTP_SERVER_ERROR_MAX).contains(&status))
}

fn same_trace_process_stream(left: &SemanticAction, right: &SemanticAction) -> bool {
    left.trace_id == right.trace_id
        && left.process == right.process
        && left
            .attributes
            .get(PAYLOAD_STREAM_KEY_ATTR)
            .zip(right.attributes.get(STREAM_KEY_ATTR))
            .is_some_and(|(left, right)| left == right)
        && http_stream_ids_match(left, right)
}

fn http_stream_ids_match(request: &SemanticAction, http_message: &SemanticAction) -> bool {
    match (
        request.attributes.get(HTTP_REQUEST_STREAM_ID_ATTR),
        http_message.attributes.get(HTTP_MESSAGE_STREAM_ID_ATTR),
    ) {
        (Some(request_stream_id), Some(message_stream_id)) => {
            request_stream_id == message_stream_id
        }
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get(PAYLOAD_SEQUENCE_ATTR)?.parse().ok()
}

fn http_payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get("payload_sequence")?.parse().ok()
}

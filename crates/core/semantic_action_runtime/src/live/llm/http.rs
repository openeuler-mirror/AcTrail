use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use crate::live::actions::{append_missing_evidence, llm_call_action_id_from_request_action_id};

const DIRECTION_ATTR: &str = "direction";
const DIRECTION_INBOUND: &str = "inbound";
const HTTP_RESPONSE_ACTION_ID_ATTR: &str = attrs::llm_call::HTTP_RESPONSE_ACTION_ID;
const HTTP_RESPONSE_STATUS_CODE_ATTR: &str = attrs::http_response::STATUS_CODE;
const HTTP_REQUEST_STREAM_ID_ATTR: &str = attrs::http_request::STREAM_ID;
const PAYLOAD_SEQUENCE_ATTR: &str = attrs::payload::SEQUENCE;
const PAYLOAD_STREAM_KEY_ATTR: &str = attrs::payload::STREAM_KEY;
const STATUS_CODE_ATTR: &str = "status_code";
const STREAM_KEY_ATTR: &str = "stream_key";
const HTTP_MESSAGE_STREAM_ID_ATTR: &str = "stream_id";
const HTTP_CLIENT_ERROR_MIN: u16 = 400;
const HTTP_SERVER_ERROR_MAX: u16 = 599;

pub(super) fn failed_call_for_http_response(
    http_response: &SemanticAction,
    emitted_actions: &[&SemanticAction],
) -> Option<SemanticAction> {
    if !http_error_response(http_response) {
        return None;
    }
    let response_sequence = http_payload_sequence(http_response)?;
    let request =
        latest_request_for_http_response(http_response, response_sequence, emitted_actions)?;
    let call_id = llm_call_action_id_from_request_action_id(&request.action_id);
    let call = emitted_actions
        .iter()
        .copied()
        .find(|action| action.kind == SemanticActionKind::LlmCall && action.action_id == call_id)?;
    if call.status != SemanticActionStatus::InProgress
        || call
            .attributes
            .contains_key(attrs::llm_call::RESPONSE_ACTION_ID)
    {
        return None;
    }

    let mut failed = call.clone();
    failed.status = SemanticActionStatus::Error;
    failed.completeness = SemanticActionCompleteness::Complete;
    failed.end_time = Some(http_response.end_time.unwrap_or(http_response.start_time));
    failed.attributes.insert(
        HTTP_RESPONSE_ACTION_ID_ATTR.to_string(),
        http_response.action_id.clone(),
    );
    if let Some(status_code) = http_response.attributes.get(STATUS_CODE_ATTR) {
        failed.attributes.insert(
            HTTP_RESPONSE_STATUS_CODE_ATTR.to_string(),
            status_code.clone(),
        );
    }
    if let Some(reason) = http_response.attributes.get("reason") {
        failed
            .attributes
            .insert(attrs::http_response::REASON.to_string(), reason.clone());
    }
    append_missing_evidence(&mut failed.evidence, &http_response.evidence);
    Some(failed)
}

fn latest_request_for_http_response<'a>(
    http_response: &SemanticAction,
    response_sequence: u64,
    emitted_actions: &'a [&'a SemanticAction],
) -> Option<&'a SemanticAction> {
    emitted_actions
        .iter()
        .copied()
        .filter(|action| request_matches_http_response(action, http_response, response_sequence))
        .max_by(|left, right| {
            (
                payload_sequence(left).unwrap_or_default(),
                left.action_id.as_str(),
            )
                .cmp(&(
                    payload_sequence(right).unwrap_or_default(),
                    right.action_id.as_str(),
                ))
        })
}

fn request_matches_http_response(
    request: &SemanticAction,
    http_response: &SemanticAction,
    response_sequence: u64,
) -> bool {
    request.kind == SemanticActionKind::LlmRequest
        && request.trace_id == http_response.trace_id
        && request.process == http_response.process
        && request
            .attributes
            .get(PAYLOAD_STREAM_KEY_ATTR)
            .zip(http_response.attributes.get(STREAM_KEY_ATTR))
            .is_some_and(|(left, right)| left == right)
        && http_stream_ids_match(request, http_response)
        && request.start_time <= http_response.start_time
        && payload_sequence(request).is_some_and(|sequence| sequence <= response_sequence)
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

fn http_error_response(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::HttpMessage
        && action.attributes.get(DIRECTION_ATTR).map(String::as_str) == Some(DIRECTION_INBOUND)
        && action
            .attributes
            .get(STATUS_CODE_ATTR)
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(|status| (HTTP_CLIENT_ERROR_MIN..=HTTP_SERVER_ERROR_MAX).contains(&status))
}

fn http_payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get("payload_sequence")?.parse().ok()
}

fn payload_sequence(action: &SemanticAction) -> Option<u64> {
    action.attributes.get(PAYLOAD_SEQUENCE_ATTR)?.parse().ok()
}

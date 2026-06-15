use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
    SemanticEvidenceKind,
};

const LINK_VALID_ATTR: &str = "actrail.link.valid";
const VALID_FALSE: &str = "false";
const PROCESS_PARENT_IDENTITY_STATE_ATTR: &str = "process.parent.identity_state";
const PROCESS_PARENT_IDENTITY_STATE_CONFLICT: &str = "conflict";
const DIRECTION_ATTR: &str = "direction";
const DIRECTION_INBOUND: &str = "inbound";
const DIRECTION_OUTBOUND: &str = "outbound";
const PAYLOAD_SEGMENT_ID_ATTR: &str = "payload_segment_id";
const PAYLOAD_SEQUENCE_ATTR: &str = "payload_sequence";
const PAYLOAD_STREAM_KEY_ATTR: &str = "payload.stream_key";
const STREAM_KEY_ATTR: &str = "stream_key";
const HTTP_RESPONSE_STATUS_CODE_ATTR: &str = "http.response.status_code";
const HTTP_RESPONSE_STREAM_ID_ATTR: &str = "http.response.stream_id";
const STATUS_CODE_ATTR: &str = "status_code";
const PAYLOAD_SEQUENCE_LLM_ATTR: &str = "payload.sequence";
const HTTP_MESSAGE_STREAM_ID_ATTR: &str = "stream_id";

pub(super) fn invalid_link(
    link: &SemanticActionLink,
    parent: &SemanticAction,
    child: &SemanticAction,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> bool {
    link.attributes
        .get(LINK_VALID_ATTR)
        .is_some_and(|value| value == VALID_FALSE)
        || invalid_parent_identity_link(link, child)
        || invalid_response_http_link(link, parent, child, action_by_id)
}

fn invalid_parent_identity_link(link: &SemanticActionLink, child: &SemanticAction) -> bool {
    (link.role == SemanticActionLinkRole::AgentPerformedAction
        || link.role == SemanticActionLinkRole::CommandContainsCommandInvocation)
        && child
            .attributes
            .get(PROCESS_PARENT_IDENTITY_STATE_ATTR)
            .is_some_and(|value| value == PROCESS_PARENT_IDENTITY_STATE_CONFLICT)
}

fn invalid_response_http_link(
    link: &SemanticActionLink,
    parent: &SemanticAction,
    child: &SemanticAction,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> bool {
    if link.role != SemanticActionLinkRole::LlmResponseHttpMessage {
        return false;
    }
    if actions_share_payload_segment(parent, child) {
        return false;
    }
    if !response_stream_candidate(parent, child) {
        return true;
    }
    let Some(response_marker) = response_payload_marker(parent) else {
        return true;
    };
    let Some(child_marker) = http_payload_marker(child) else {
        return true;
    };
    if child_marker > response_marker || http_request_between(child, response_marker, action_by_id)
    {
        return true;
    }
    best_response_http_message(parent, response_marker, action_by_id)
        .is_none_or(|best| best.action_id != child.action_id)
}

fn best_response_http_message<'a>(
    llm_response: &SemanticAction,
    response_marker: u64,
    action_by_id: &'a BTreeMap<String, SemanticAction>,
) -> Option<&'a SemanticAction> {
    action_by_id
        .values()
        .filter(|candidate| response_stream_candidate(llm_response, candidate))
        .filter_map(|candidate| Some((http_payload_marker(candidate)?, candidate)))
        .filter(|(marker, _)| *marker <= response_marker)
        .filter(|(_, candidate)| !http_request_between(candidate, response_marker, action_by_id))
        .max_by(|left, right| {
            (left.0, left.1.action_id.as_str()).cmp(&(right.0, right.1.action_id.as_str()))
        })
        .map(|(_, candidate)| candidate)
}

fn http_request_between(
    http_response: &SemanticAction,
    response_marker: u64,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> bool {
    let Some(response_message_marker) = http_payload_marker(http_response) else {
        return false;
    };
    action_by_id.values().any(|candidate| {
        candidate.attributes.get(DIRECTION_ATTR).map(String::as_str) == Some(DIRECTION_OUTBOUND)
            && same_trace_process_stream(http_response, candidate)
            && http_payload_marker(candidate)
                .is_some_and(|marker| response_message_marker < marker && marker < response_marker)
    })
}

fn response_stream_candidate(parent: &SemanticAction, child: &SemanticAction) -> bool {
    parent.kind == SemanticActionKind::LlmResponse
        && child.kind == SemanticActionKind::HttpMessage
        && parent.trace_id == child.trace_id
        && parent.process == child.process
        && child.attributes.get(DIRECTION_ATTR).map(String::as_str) == Some(DIRECTION_INBOUND)
        && parent
            .attributes
            .get(PAYLOAD_STREAM_KEY_ATTR)
            .zip(child.attributes.get(STREAM_KEY_ATTR))
            .is_some_and(|(left, right)| left == right)
        && http_stream_ids_match(parent, child)
        && response_status_codes_are_compatible(parent, child)
}

fn http_stream_ids_match(response: &SemanticAction, http_message: &SemanticAction) -> bool {
    match (
        response.attributes.get(HTTP_RESPONSE_STREAM_ID_ATTR),
        http_message.attributes.get(HTTP_MESSAGE_STREAM_ID_ATTR),
    ) {
        (Some(response_stream_id), Some(message_stream_id)) => {
            response_stream_id == message_stream_id
        }
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn response_status_codes_are_compatible(parent: &SemanticAction, child: &SemanticAction) -> bool {
    match (
        parent.attributes.get(HTTP_RESPONSE_STATUS_CODE_ATTR),
        child.attributes.get(STATUS_CODE_ATTR),
    ) {
        (Some(left), Some(right)) => left == right,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn same_trace_process_stream(left: &SemanticAction, right: &SemanticAction) -> bool {
    left.trace_id == right.trace_id
        && left.process == right.process
        && left
            .attributes
            .get(STREAM_KEY_ATTR)
            .zip(right.attributes.get(STREAM_KEY_ATTR))
            .is_some_and(|(left, right)| left == right)
}

fn actions_share_payload_segment(parent: &SemanticAction, child: &SemanticAction) -> bool {
    let Some(payload_segment_id) = http_payload_segment_id(child) else {
        return false;
    };
    parent.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment && evidence.id == payload_segment_id
    })
}

fn response_payload_marker(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(PAYLOAD_SEQUENCE_LLM_ATTR)
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            action
                .evidence
                .iter()
                .filter(|evidence| evidence.kind == SemanticEvidenceKind::PayloadSegment)
                .map(|evidence| evidence.id)
                .min()
        })
}

fn http_payload_marker(action: &SemanticAction) -> Option<u64> {
    action
        .attributes
        .get(PAYLOAD_SEQUENCE_ATTR)
        .and_then(|value| value.parse().ok())
        .or_else(|| http_payload_segment_id(action))
}

fn http_payload_segment_id(action: &SemanticAction) -> Option<u64> {
    action.attributes.get(PAYLOAD_SEGMENT_ID_ATTR)?.parse().ok()
}

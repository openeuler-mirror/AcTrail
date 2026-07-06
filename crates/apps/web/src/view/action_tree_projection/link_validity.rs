use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
    SemanticEvidenceKind, attr_keys as attrs,
};

const LINK_VALID_ATTR: &str = attrs::actrail::LINK_VALID;
const VALID_FALSE: &str = "false";
const PROCESS_PARENT_IDENTITY_STATE_ATTR: &str = attrs::process_parent::IDENTITY_STATE;
const PROCESS_PARENT_IDENTITY_STATE_CONFLICT: &str = "conflict";
const DIRECTION_ATTR: &str = "direction";
const DIRECTION_INBOUND: &str = "inbound";
const DIRECTION_OUTBOUND: &str = "outbound";
const PAYLOAD_SEGMENT_ID_ATTR: &str = "payload_segment_id";
const PAYLOAD_SEQUENCE_ATTR: &str = "payload_sequence";
const PAYLOAD_STREAM_KEY_ATTR: &str = attrs::payload::STREAM_KEY;
const STREAM_KEY_ATTR: &str = "stream_key";
const HTTP_RESPONSE_STATUS_CODE_ATTR: &str = attrs::http_response::STATUS_CODE;
const HTTP_RESPONSE_STREAM_ID_ATTR: &str = attrs::http_response::STREAM_ID;
const STATUS_CODE_ATTR: &str = "status_code";
const PAYLOAD_SEQUENCE_LLM_ATTR: &str = attrs::payload::SEQUENCE;
const PAYLOAD_SEQUENCE_START_LLM_ATTR: &str = attrs::payload::SEQUENCE_START;
const PAYLOAD_SEQUENCE_END_LLM_ATTR: &str = attrs::payload::SEQUENCE_END;
const HTTP_MESSAGE_STREAM_ID_ATTR: &str = "stream_id";

pub(super) fn invalid_link(
    link: &SemanticActionLink,
    parent: &SemanticAction,
    child: &SemanticAction,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> bool {
    !link.valid
        || link
            .attributes
            .get(LINK_VALID_ATTR)
            .is_some_and(|value| value == VALID_FALSE)
        || invalid_llm_call_child_link(link, parent, child)
        || invalid_mcp_child_link(link, parent, child)
        || invalid_parent_identity_link(link, child)
        || invalid_response_http_link(link, parent, child, action_by_id)
}

fn invalid_llm_call_child_link(
    link: &SemanticActionLink,
    parent: &SemanticAction,
    child: &SemanticAction,
) -> bool {
    match link.role {
        SemanticActionLinkRole::LlmCallRequest => {
            parent.kind != SemanticActionKind::LlmCall
                || child.kind != SemanticActionKind::LlmRequest
                || !call_references_action(parent, attrs::llm_call::REQUEST_ACTION_ID, child)
        }
        SemanticActionLinkRole::LlmCallResponse => {
            parent.kind != SemanticActionKind::LlmCall
                || child.kind != SemanticActionKind::LlmResponse
                || !call_references_action(parent, attrs::llm_call::RESPONSE_ACTION_ID, child)
        }
        _ => false,
    }
}

fn call_references_action(call: &SemanticAction, attr: &str, child: &SemanticAction) -> bool {
    call.trace_id == child.trace_id
        && call.process == child.process
        && call
            .attributes
            .get(attr)
            .is_some_and(|action_id| action_id == &child.action_id)
}

fn invalid_mcp_child_link(
    link: &SemanticActionLink,
    parent: &SemanticAction,
    child: &SemanticAction,
) -> bool {
    match link.role {
        SemanticActionLinkRole::McpToolCallRequest => {
            parent.kind != SemanticActionKind::McpToolCall
                || child.kind != SemanticActionKind::McpRequest
                || !mcp_child_references_parent(
                    child,
                    attrs::mcp::TOOL_CALL_ACTION_ID,
                    parent,
                    &format!("{}:request", parent.action_id),
                )
        }
        SemanticActionLinkRole::McpToolCallResponse => {
            parent.kind != SemanticActionKind::McpToolCall
                || child.kind != SemanticActionKind::McpResponse
                || !mcp_child_references_parent(
                    child,
                    attrs::mcp::TOOL_CALL_ACTION_ID,
                    parent,
                    &format!("{}:response", parent.action_id),
                )
        }
        SemanticActionLinkRole::McpRequestStdout => {
            parent.kind != SemanticActionKind::McpRequest
                || child.kind != SemanticActionKind::McpStdout
                || !mcp_child_references_parent(
                    child,
                    attrs::mcp::REQUEST_ACTION_ID,
                    parent,
                    &mcp_sibling_action_id(&parent.action_id, "request", "stdout"),
                )
        }
        SemanticActionLinkRole::McpRequestClientSend => {
            parent.kind != SemanticActionKind::McpRequest
                || child.kind != SemanticActionKind::McpClientSend
                || !mcp_child_references_parent(
                    child,
                    attrs::mcp::REQUEST_ACTION_ID,
                    parent,
                    &mcp_sibling_action_id(&parent.action_id, "request", "client_send"),
                )
        }
        SemanticActionLinkRole::McpResponseStdin => {
            parent.kind != SemanticActionKind::McpResponse
                || child.kind != SemanticActionKind::McpStdin
                || !mcp_child_references_parent(
                    child,
                    attrs::mcp::RESPONSE_ACTION_ID,
                    parent,
                    &mcp_sibling_action_id(&parent.action_id, "response", "stdin"),
                )
        }
        SemanticActionLinkRole::McpResponseClientReceive => {
            parent.kind != SemanticActionKind::McpResponse
                || child.kind != SemanticActionKind::McpClientReceive
                || !mcp_child_references_parent(
                    child,
                    attrs::mcp::RESPONSE_ACTION_ID,
                    parent,
                    &mcp_sibling_action_id(&parent.action_id, "response", "client_receive"),
                )
        }
        _ => false,
    }
}

fn mcp_child_references_parent(
    child: &SemanticAction,
    attr: &str,
    parent: &SemanticAction,
    fallback_child_action_id: &str,
) -> bool {
    child
        .attributes
        .get(attr)
        .is_some_and(|action_id| action_id == &parent.action_id)
        || child.action_id == fallback_child_action_id
}

fn mcp_sibling_action_id(action_id: &str, from_suffix: &str, to_suffix: &str) -> String {
    let parent = action_id
        .strip_suffix(&format!(":{from_suffix}"))
        .unwrap_or(action_id);
    format!("{parent}:{to_suffix}")
}

fn invalid_parent_identity_link(link: &SemanticActionLink, child: &SemanticAction) -> bool {
    (link.role == SemanticActionLinkRole::AgentPerformedAction
        || link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
        || link.role == SemanticActionLinkRole::CommandContainsMcpToolCall)
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
    }) || payload_aggregate_matches_http_message(parent, child)
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

fn payload_aggregate_matches_http_message(
    llm_action: &SemanticAction,
    http_message: &SemanticAction,
) -> bool {
    if !llm_action
        .evidence
        .iter()
        .any(|evidence| evidence.kind == SemanticEvidenceKind::PayloadAggregate)
    {
        return false;
    }
    if llm_action
        .attributes
        .get(PAYLOAD_STREAM_KEY_ATTR)
        .zip(http_message.attributes.get(STREAM_KEY_ATTR))
        .is_none_or(|(left, right)| left != right)
    {
        return false;
    }
    let Some(http_sequence) = http_payload_marker(http_message) else {
        return false;
    };
    payload_sequence_range(llm_action)
        .is_some_and(|(start, end)| start <= http_sequence && http_sequence <= end)
}

fn payload_sequence_range(action: &SemanticAction) -> Option<(u64, u64)> {
    let start = action
        .attributes
        .get(PAYLOAD_SEQUENCE_START_LLM_ATTR)?
        .parse()
        .ok()?;
    let end = action
        .attributes
        .get(PAYLOAD_SEQUENCE_END_LLM_ATTR)?
        .parse()
        .ok()?;
    Some((start, end))
}

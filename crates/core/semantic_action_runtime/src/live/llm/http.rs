use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

const DIRECTION_ATTR: &str = "direction";
const DIRECTION_INBOUND: &str = "inbound";
const HTTP_REQUEST_STREAM_ID_ATTR: &str = attrs::http_request::STREAM_ID;
const PAYLOAD_SEQUENCE_ATTR: &str = attrs::payload::SEQUENCE;
const PAYLOAD_STREAM_KEY_ATTR: &str = attrs::payload::STREAM_KEY;
const STATUS_CODE_ATTR: &str = "status_code";
const STREAM_KEY_ATTR: &str = "stream_key";
const HTTP_MESSAGE_STREAM_ID_ATTR: &str = "stream_id";
const HTTP_CLIENT_ERROR_MIN: u16 = 400;
const HTTP_SERVER_ERROR_MAX: u16 = 599;

pub(super) fn failed_response_for_open_request(
    http_response: &SemanticAction,
    request: &SemanticAction,
    call: &SemanticAction,
) -> Option<SemanticAction> {
    if !error_response(http_response) {
        return None;
    }
    let response_sequence = http_payload_sequence(http_response)?;
    if !request_matches_http_response(request, http_response, response_sequence) {
        return None;
    }
    if call.status != SemanticActionStatus::InProgress
        || call
            .attributes
            .contains_key(attrs::llm_call::RESPONSE_ACTION_ID)
    {
        return None;
    }

    let mut attributes = BTreeMap::new();
    attributes.insert(attrs::llm_response::STREAM.to_string(), "false".to_string());
    attributes.insert(attrs::llm_response::DONE.to_string(), "true".to_string());
    attributes.insert(
        attrs::llm_response::CHUNK_COUNT.to_string(),
        "0".to_string(),
    );
    attributes.insert(
        attrs::llm_response::BODY_FORMAT.to_string(),
        "http_error".to_string(),
    );
    if let Some(content_length) = http_response.attributes.get("content_length") {
        attributes.insert(
            attrs::llm_response::PAYLOAD_BYTES.to_string(),
            content_length.clone(),
        );
    }
    copy_request_attr_as(
        request,
        &mut attributes,
        attrs::llm_request::MODEL,
        attrs::llm_response::MODEL,
    );
    copy_request_attr(request, &mut attributes, attrs::url::SCHEME);
    copy_request_attr(request, &mut attributes, attrs::url::PATH);
    copy_request_attr(request, &mut attributes, attrs::server::ADDRESS);
    copy_http_attr(
        http_response,
        &mut attributes,
        "status_code",
        attrs::http_response::STATUS_CODE,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "reason",
        attrs::http_response::REASON,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "stream_id",
        attrs::http_response::STREAM_ID,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "stream_key",
        attrs::payload::STREAM_KEY,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "payload_sequence",
        attrs::payload::SEQUENCE,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "payload_sequence",
        attrs::payload::SEQUENCE_START,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "payload_sequence",
        attrs::payload::SEQUENCE_END,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        "source_boundary",
        attrs::payload::SOURCE_BOUNDARY,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        attrs::network::PROTOCOL_NAME,
        attrs::network::PROTOCOL_NAME,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        attrs::network::PROTOCOL_VERSION,
        attrs::network::PROTOCOL_VERSION,
    );
    copy_http_attr(
        http_response,
        &mut attributes,
        attrs::network::PROTOCOL_VERSION,
        attrs::http_response::PROTOCOL,
    );

    let status = attributes
        .get(attrs::http_response::STATUS_CODE)
        .cloned()
        .unwrap_or_else(|| "HTTP error".to_string());
    Some(SemanticAction {
        action_id: failed_response_action_id(http_response),
        trace_id: http_response.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: format!("LLM response HTTP {status}"),
        start_time: http_response.start_time,
        end_time: http_response.end_time.or(Some(http_response.start_time)),
        process: http_response.process.clone(),
        status: SemanticActionStatus::Error,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: http_response.evidence.clone(),
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
        && request_precedes_response(request, http_response, response_sequence)
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

pub(super) fn error_response(action: &SemanticAction) -> bool {
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

fn failed_response_action_id(http_response: &SemanticAction) -> String {
    format!("{}:llm.response", http_response.action_id)
}

fn copy_request_attr(
    request: &SemanticAction,
    attributes: &mut BTreeMap<String, String>,
    key: &'static str,
) {
    if let Some(value) = request.attributes.get(key) {
        attributes.insert(key.to_string(), value.clone());
    }
}

fn copy_request_attr_as(
    request: &SemanticAction,
    attributes: &mut BTreeMap<String, String>,
    source_key: &'static str,
    target_key: &'static str,
) {
    if let Some(value) = request.attributes.get(source_key) {
        attributes.insert(target_key.to_string(), value.clone());
    }
}

fn copy_http_attr(
    http_response: &SemanticAction,
    attributes: &mut BTreeMap<String, String>,
    source_key: &'static str,
    target_key: &'static str,
) {
    if let Some(value) = http_response.attributes.get(source_key) {
        attributes.insert(target_key.to_string(), value.clone());
    }
}

fn request_precedes_response(
    request: &SemanticAction,
    response: &SemanticAction,
    response_sequence: u64,
) -> bool {
    if request.start_time > response.start_time {
        return false;
    }
    payload_sequence(request).is_some_and(|sequence| sequence <= response_sequence)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, SystemTime};

    use model_core::ids::TraceId;
    use model_core::process::ProcessIdentity;

    use super::*;

    #[test]
    fn request_before_response_with_later_sequence_does_not_match() {
        let request = request_action(SystemTime::UNIX_EPOCH, 30);
        let response = response_action(SystemTime::UNIX_EPOCH + Duration::from_secs(1));

        assert!(!request_matches_http_response(&request, &response, 20));
    }

    #[test]
    fn request_before_response_with_earlier_sequence_matches() {
        let request = request_action(SystemTime::UNIX_EPOCH, 20);
        let response = response_action(SystemTime::UNIX_EPOCH + Duration::from_secs(1));

        assert!(request_matches_http_response(&request, &response, 20));
    }

    #[test]
    fn request_after_response_does_not_match_even_with_earlier_sequence() {
        let request = request_action(SystemTime::UNIX_EPOCH + Duration::from_secs(2), 10);
        let response = response_action(SystemTime::UNIX_EPOCH + Duration::from_secs(1));

        assert!(!request_matches_http_response(&request, &response, 20));
    }

    fn request_action(start_time: SystemTime, payload_sequence: u64) -> SemanticAction {
        let mut attributes = BTreeMap::new();
        attributes.insert(PAYLOAD_STREAM_KEY_ATTR.to_string(), "stream-a".to_string());
        attributes.insert(HTTP_REQUEST_STREAM_ID_ATTR.to_string(), "1".to_string());
        attributes.insert(
            PAYLOAD_SEQUENCE_ATTR.to_string(),
            payload_sequence.to_string(),
        );
        semantic_action(
            "request",
            SemanticActionKind::LlmRequest,
            start_time,
            attributes,
        )
    }

    fn response_action(start_time: SystemTime) -> SemanticAction {
        let mut attributes = BTreeMap::new();
        attributes.insert(STREAM_KEY_ATTR.to_string(), "stream-a".to_string());
        attributes.insert(HTTP_MESSAGE_STREAM_ID_ATTR.to_string(), "1".to_string());
        semantic_action(
            "response",
            SemanticActionKind::HttpMessage,
            start_time,
            attributes,
        )
    }

    fn semantic_action(
        action_id: &str,
        kind: SemanticActionKind,
        start_time: SystemTime,
        attributes: BTreeMap<String, String>,
    ) -> SemanticAction {
        SemanticAction {
            action_id: action_id.to_string(),
            trace_id: TraceId::new(7),
            kind,
            title: action_id.to_string(),
            start_time,
            end_time: None,
            process: ProcessIdentity::new(4242, 1, 0),
            status: SemanticActionStatus::InProgress,
            completeness: SemanticActionCompleteness::Partial,
            confidence_millis: None,
            attributes,
            evidence: Vec::new(),
        }
    }
}

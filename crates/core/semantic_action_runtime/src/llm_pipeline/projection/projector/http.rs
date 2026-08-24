//! HTTP-aware failure and incomplete-response projection.

use std::collections::BTreeMap;

use model_core::payload::PayloadSegment;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind, attr_keys as attrs, evidence_roles,
};

const DIRECTION_ATTR: &str = "direction";
const DIRECTION_INBOUND: &str = "inbound";
const CONTENT_TYPE_ATTR: &str = "content_type";
const STATUS_CODE_ATTR: &str = "status_code";
const HTTP_SUCCESS_MIN: u16 = 200;
const HTTP_SUCCESS_MAX: u16 = 299;
const HTTP_CLIENT_ERROR_MIN: u16 = 400;
const HTTP_SERVER_ERROR_MAX: u16 = 599;

enum HttpResponseFailure {
    ErrorStatus,
    UnexpectedHtml,
}

impl HttpResponseFailure {
    fn classify(action: &SemanticAction) -> Option<Self> {
        if action.kind != SemanticActionKind::HttpMessage
            || action.attributes.get(DIRECTION_ATTR).map(String::as_str) != Some(DIRECTION_INBOUND)
        {
            return None;
        }
        let status = action
            .attributes
            .get(STATUS_CODE_ATTR)?
            .parse::<u16>()
            .ok()?;
        if (HTTP_CLIENT_ERROR_MIN..=HTTP_SERVER_ERROR_MAX).contains(&status) {
            return Some(Self::ErrorStatus);
        }
        let media_type = action
            .attributes
            .get(CONTENT_TYPE_ATTR)?
            .split(';')
            .next()?
            .trim();
        ((HTTP_SUCCESS_MIN..=HTTP_SUCCESS_MAX).contains(&status)
            && media_type.eq_ignore_ascii_case("text/html"))
        .then_some(Self::UnexpectedHtml)
    }

    fn body_format(&self) -> &'static str {
        match self {
            Self::ErrorStatus => "http_error",
            Self::UnexpectedHtml => "unexpected_html",
        }
    }

    fn title(&self, status: &str) -> String {
        match self {
            Self::ErrorStatus => format!("LLM response HTTP {status}"),
            Self::UnexpectedHtml => {
                format!("LLM response HTTP {status} unexpected text/html")
            }
        }
    }
}

pub(in crate::llm_pipeline) fn failed_response_for_open_request(
    http_response: &SemanticAction,
    request: &SemanticAction,
    call: &SemanticAction,
) -> Option<SemanticAction> {
    let failure = HttpResponseFailure::classify(http_response)?;
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
        failure.body_format().to_string(),
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
        title: failure.title(&status),
        start_time: http_response.start_time,
        end_time: http_response.end_time.or(Some(http_response.start_time)),
        process: http_response.process.clone(),
        status: SemanticActionStatus::Error,
        completeness: SemanticActionCompleteness::Complete,
        attributes,
        evidence: http_response.evidence.clone(),
    })
}

pub(in crate::llm_pipeline) fn terminal_failure_response(action: &SemanticAction) -> bool {
    HttpResponseFailure::classify(action).is_some()
}

pub(in crate::llm_pipeline) fn damaged_response_for_open_request(
    http_response: Option<&SemanticAction>,
    segment: &PayloadSegment,
    request: &SemanticAction,
) -> SemanticAction {
    let mut attributes = BTreeMap::new();
    attributes.insert(attrs::llm_response::STREAM.to_string(), "false".to_string());
    attributes.insert(attrs::llm_response::DONE.to_string(), "false".to_string());
    attributes.insert(
        attrs::llm_response::BODY_FORMAT.to_string(),
        "capture_incomplete".to_string(),
    );
    attributes.insert(
        attrs::llm_response::PAYLOAD_BYTES.to_string(),
        segment.captured_size.to_string(),
    );
    copy_request_attr_as(
        request,
        &mut attributes,
        attrs::llm_request::MODEL,
        attrs::llm_response::MODEL,
    );
    copy_request_attr(request, &mut attributes, attrs::url::SCHEME);
    copy_request_attr(request, &mut attributes, attrs::url::PATH);
    copy_request_attr(request, &mut attributes, attrs::server::ADDRESS);
    attributes.insert(
        attrs::payload::STREAM_KEY.to_string(),
        segment.stream_key.to_string(),
    );
    attributes.insert(
        attrs::payload::OPERATION_ID.to_string(),
        segment.operation_id.to_string(),
    );
    attributes.insert(
        attrs::payload::SEQUENCE.to_string(),
        segment.operation_id.to_string(),
    );
    attributes.insert(
        attrs::payload::SEQUENCE_START.to_string(),
        segment.operation_id.to_string(),
    );
    attributes.insert(
        attrs::payload::SEQUENCE_END.to_string(),
        segment.operation_id.to_string(),
    );
    if let Some(http_response) = http_response {
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
            attrs::network::PROTOCOL_VERSION,
            attrs::http_response::PROTOCOL,
        );
    }
    let action_id = http_response
        .map(failed_response_action_id)
        .unwrap_or_else(|| {
            format!(
                "{}:operation:{}:damaged:llm.response",
                request.action_id, segment.operation_id
            )
        });
    let evidence = http_response
        .map(|response| response.evidence.clone())
        .unwrap_or_else(|| {
            vec![SemanticEvidence {
                kind: SemanticEvidenceKind::PayloadSegment,
                id: segment.segment_id.get(),
                role: evidence_roles::llm_response::PAYLOAD.to_string(),
            }]
        });
    SemanticAction {
        action_id,
        trace_id: segment.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: "LLM response capture incomplete".to_string(),
        start_time: http_response
            .map(|response| response.start_time)
            .unwrap_or(segment.observed_at),
        end_time: Some(segment.observed_at),
        process: segment.process.clone(),
        status: SemanticActionStatus::Error,
        completeness: SemanticActionCompleteness::Partial,
        attributes,
        evidence,
    }
}

pub(in crate::llm_pipeline) fn mark_response_for_incomplete_request(
    response: &mut SemanticAction,
    http_response: &SemanticAction,
) {
    response.title = "LLM response for incomplete request capture".to_string();
    response.status = SemanticActionStatus::Error;
    response.end_time = response.end_time.or(Some(http_response.start_time));
    response.attributes.insert(
        attrs::llm_response::BODY_FORMAT.to_string(),
        "request_capture_incomplete".to_string(),
    );
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

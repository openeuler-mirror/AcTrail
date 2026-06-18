//! LLM request projection from split HTTP payloads.

use std::collections::BTreeMap;

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::{
    PayloadOperationCompletionState, PayloadSegment, PayloadSourceBoundary, PayloadTruncationState,
};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs, evidence_roles,
};
use serde_json::Value;

use crate::payload_projection::http::HttpRequestParts;

use super::evidence::{insert_payload_span_attributes, payload_aggregate_evidence};
use super::stream::PayloadStreamGroupKey;

pub(super) fn project_stream_llm_request_action(
    config: &SemanticRetentionConfig,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpRequestParts,
    segments: &[&PayloadSegment],
) -> Option<SemanticAction> {
    let body = parse_llm_request_body(&http.body)?;
    let first = *segments.first()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let attributes = llm_attributes(config, segments, raw_bytes, &http, &body);
    let evidence = payload_aggregate_evidence(segments, evidence_roles::llm_request::PAYLOAD);
    Some(SemanticAction {
        action_id: llm_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmRequest,
        title: llm_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status: llm_status(segments),
        completeness: llm_stream_completeness(segments),
        confidence_millis: None,
        attributes,
        evidence,
    })
}

fn llm_attributes(
    config: &SemanticRetentionConfig,
    segments: &[&PayloadSegment],
    raw_bytes: &[u8],
    http: &HttpRequestParts,
    body: &LlmRequestBody,
) -> BTreeMap<String, String> {
    let first = segments[0];
    let mut attributes = BTreeMap::new();
    attributes.insert(
        attrs::llm_request::RAW_PAYLOAD_BYTES.to_string(),
        raw_bytes.len().to_string(),
    );
    attributes.insert(
        attrs::llm_request::PAYLOAD_BYTES.to_string(),
        http.body.len().to_string(),
    );
    if body.json_valid {
        attributes.insert(
            attrs::http_request::BODY_JSON_STATE.to_string(),
            "valid".to_string(),
        );
    } else {
        attributes.insert(
            attrs::http_request::BODY_JSON_STATE.to_string(),
            "invalid_or_unavailable".to_string(),
        );
    }
    if http.body.contains(&0) {
        attributes.insert(
            attrs::http_request::BODY_CONTAINS_NUL.to_string(),
            "true".to_string(),
        );
    }
    attributes.insert(
        attrs::http_request::PROTOCOL.to_string(),
        http.protocol.to_string(),
    );
    attributes.insert(
        attrs::network::PROTOCOL_NAME.to_string(),
        "http".to_string(),
    );
    attributes.insert(
        attrs::network::PROTOCOL_VERSION.to_string(),
        http.protocol.to_string(),
    );
    attributes.insert(attrs::url::SCHEME.to_string(), http.scheme.to_string());
    if let Some(method) = &http.method {
        attributes.insert(attrs::http_request::METHOD.to_string(), method.clone());
    }
    if let Some(authority) = &http.authority {
        attributes.insert(attrs::server::ADDRESS.to_string(), authority.clone());
    }
    if let Some(path) = &http.path {
        attributes.insert(attrs::url::PATH.to_string(), path.clone());
    }
    if let Some(stream_id) = http.stream_id {
        attributes.insert(
            attrs::http_request::STREAM_ID.to_string(),
            stream_id.to_string(),
        );
    }
    if let Some(headers) = &http.headers_text {
        attributes.insert(
            attrs::http_request::HEADERS_TEXT.to_string(),
            headers.clone(),
        );
        attributes.insert(
            attrs::http_request::HEADERS_ENCODING.to_string(),
            "text".to_string(),
        );
    }
    if let Some(headers) = &http.headers_hpack_base64 {
        attributes.insert(
            attrs::http_request::HEADERS_HPACK_BASE64.to_string(),
            headers.clone(),
        );
        attributes.insert(
            attrs::http_request::HEADERS_ENCODING.to_string(),
            "hpack".to_string(),
        );
    }
    if let Some(model) = body.model.as_deref() {
        attributes.insert(attrs::llm_request::MODEL.to_string(), model.to_string());
    }
    if config.llm_request_full_provider_json_enabled() {
        if let Some(body_json) = body.body_json.as_deref() {
            attributes.insert(
                attrs::llm_request::BODY_JSON.to_string(),
                body_json.to_string(),
            );
        } else if let Some(body_text) = body.body_text.as_deref() {
            attributes.insert(
                attrs::llm_request::BODY_TEXT.to_string(),
                body_text.to_string(),
            );
        }
    }
    attributes.insert(
        attrs::payload::STREAM_KEY.to_string(),
        first.stream_key.to_string(),
    );
    attributes.insert(
        attrs::payload::OPERATION_ID.to_string(),
        first.operation_id.to_string(),
    );
    attributes.insert(
        attrs::payload::SEQUENCE.to_string(),
        first.sequence.to_string(),
    );
    insert_payload_span_attributes(&mut attributes, segments);
    attributes.insert(
        attrs::payload::SOURCE_BOUNDARY.to_string(),
        format!("{:?}", first.source_boundary),
    );
    attributes.insert(attrs::payload::LIBRARY.to_string(), first.library.clone());
    attributes.insert(attrs::payload::SYMBOL.to_string(), first.symbol.clone());
    attributes
}

fn plaintext_transport_scheme(source_boundary: PayloadSourceBoundary) -> &'static str {
    match source_boundary {
        PayloadSourceBoundary::TlsUserSpace => "https",
        PayloadSourceBoundary::Syscall => "http",
        PayloadSourceBoundary::Stdio => {
            unreachable!("stdio payloads are not HTTP transport candidates")
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LlmRequestBody {
    json_valid: bool,
    model: Option<String>,
    body_json: Option<String>,
    body_text: Option<String>,
}

fn parse_llm_request_body(body: &[u8]) -> Option<LlmRequestBody> {
    if let Ok(value) = serde_json::from_slice::<Value>(body)
        && json_value_is_llm_request(&value)
    {
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        return Some(LlmRequestBody {
            json_valid: true,
            model,
            body_json: Some(value.to_string()),
            body_text: None,
        });
    }
    let text = String::from_utf8_lossy(body);
    if lossy_text_is_llm_request(&text) {
        Some(LlmRequestBody {
            json_valid: false,
            model: extract_json_string_lossy(&text, "model"),
            body_json: None,
            body_text: Some(text.into_owned()),
        })
    } else {
        None
    }
}

fn json_value_is_llm_request(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("model")
        && (object.contains_key("messages")
            || object.contains_key("prompt")
            || object.contains_key("input"))
}

fn lossy_text_is_llm_request(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("\"model\"")
        && (text.contains("\"messages\"")
            || text.contains("\"prompt\"")
            || text.contains("\"input\""))
}

fn extract_json_string_lossy(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let index = text.find(&needle)?;
    let after_key = &text[index + needle.len()..];
    let colon = after_key.find(':')?;
    let mut value = after_key[colon + 1..].trim_start_matches([' ', '\t', '\r', '\n', '\0']);
    value = value.strip_prefix('"').unwrap_or(value);
    let end = value
        .find(['"', ',', '\r', '\n', '\0'])
        .unwrap_or(value.len());
    (!value[..end].is_empty()).then(|| value[..end].to_string())
}

fn llm_stream_completeness(segments: &[&PayloadSegment]) -> SemanticActionCompleteness {
    if segments
        .iter()
        .all(|segment| segment_capture_is_complete(segment))
    {
        SemanticActionCompleteness::Complete
    } else {
        SemanticActionCompleteness::Partial
    }
}

fn segment_capture_is_complete(segment: &PayloadSegment) -> bool {
    segment.truncation == PayloadTruncationState::Complete
        && segment.operation_completion_state == PayloadOperationCompletionState::Success
        && segment.operation_original_size == segment.operation_captured_size
}

fn llm_status(segments: &[&PayloadSegment]) -> SemanticActionStatus {
    if segments.iter().any(|segment| {
        segment.operation_completion_state == PayloadOperationCompletionState::Failed
    }) {
        SemanticActionStatus::Error
    } else if segments.iter().all(|segment| {
        segment.operation_completion_state == PayloadOperationCompletionState::Success
    }) {
        SemanticActionStatus::Success
    } else {
        SemanticActionStatus::Unknown
    }
}

fn llm_title(attributes: &BTreeMap<String, String>) -> String {
    attributes
        .get(attrs::llm_request::MODEL)
        .map(|model| format!("LLM request {}", model))
        .unwrap_or_else(|| "LLM request".to_string())
}

fn llm_stream_action_id(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    segment: &PayloadSegment,
) -> String {
    format!(
        "trace:{}:payload-stream:{}:{}:{}:llm.request",
        segment.trace_id.get(),
        key.process.pid,
        key.stream_key,
        message_start
    )
}

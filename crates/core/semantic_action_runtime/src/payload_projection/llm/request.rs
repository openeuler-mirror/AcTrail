//! LLM request projection from split HTTP payloads.

use std::collections::{BTreeMap, BTreeSet};

use model_core::payload::{
    PayloadOperationCompletionState, PayloadSegment, PayloadSourceBoundary, PayloadTruncationState,
};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind,
};
use serde_json::Value;

use crate::payload_projection::http::HttpRequestParts;

use super::stream::PayloadStreamGroupKey;

pub(super) fn project_stream_llm_request_action(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpRequestParts,
    segments: &[&PayloadSegment],
) -> Option<SemanticAction> {
    let body = parse_llm_request_body(&http.body)?;
    let first = *segments.first()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let attributes = llm_attributes(segments, raw_bytes, &http, &body);
    let evidence = payload_evidence(segments);
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
    segments: &[&PayloadSegment],
    raw_bytes: &[u8],
    http: &HttpRequestParts,
    body: &LlmRequestBody,
) -> BTreeMap<String, String> {
    let first = segments[0];
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "llm.request.raw_payload_bytes".to_string(),
        raw_bytes.len().to_string(),
    );
    attributes.insert(
        "llm.request.payload_bytes".to_string(),
        http.body.len().to_string(),
    );
    attributes.insert("llm.request.payload_text".to_string(), body.text.clone());
    attributes.insert("http.request.body_text".to_string(), body.text.clone());
    if let Some(body_json) = &body.json {
        attributes.insert("http.request.body_json".to_string(), body_json.to_string());
        attributes.insert(
            "http.request.body_json_state".to_string(),
            "valid".to_string(),
        );
    } else {
        attributes.insert(
            "http.request.body_json_state".to_string(),
            "invalid_or_unavailable".to_string(),
        );
    }
    if http.body.contains(&0) {
        attributes.insert(
            "http.request.body_contains_nul".to_string(),
            "true".to_string(),
        );
    }
    attributes.insert(
        "http.request.protocol".to_string(),
        http.protocol.to_string(),
    );
    attributes.insert("network.protocol.name".to_string(), "http".to_string());
    attributes.insert(
        "network.protocol.version".to_string(),
        http.protocol.to_string(),
    );
    attributes.insert("url.scheme".to_string(), http.scheme.to_string());
    if let Some(method) = &http.method {
        attributes.insert("http.request.method".to_string(), method.clone());
    }
    if let Some(authority) = &http.authority {
        attributes.insert("server.address".to_string(), authority.clone());
    }
    if let Some(path) = &http.path {
        attributes.insert("url.path".to_string(), path.clone());
    }
    if let Some(stream_id) = http.stream_id {
        attributes.insert("http.request.stream_id".to_string(), stream_id.to_string());
    }
    if let Some(headers) = &http.headers_text {
        attributes.insert("http.request.headers_text".to_string(), headers.clone());
        attributes.insert(
            "http.request.headers_encoding".to_string(),
            "text".to_string(),
        );
    }
    if let Some(headers) = &http.headers_hpack_base64 {
        attributes.insert(
            "http.request.headers_hpack_base64".to_string(),
            headers.clone(),
        );
        attributes.insert(
            "http.request.headers_encoding".to_string(),
            "hpack".to_string(),
        );
    }
    if let Some(model) = body.model.as_deref() {
        attributes.insert("llm.request.model".to_string(), model.to_string());
    }
    attributes.insert(
        "payload.stream_key".to_string(),
        first.stream_key.to_string(),
    );
    attributes.insert(
        "payload.operation_id".to_string(),
        first.operation_id.to_string(),
    );
    attributes.insert("payload.sequence".to_string(), first.sequence.to_string());
    attributes.insert(
        "payload.operation_ids".to_string(),
        payload_operation_ids(segments),
    );
    attributes.insert(
        "payload.segment_count".to_string(),
        segments.len().to_string(),
    );
    attributes.insert(
        "payload.source_boundary".to_string(),
        format!("{:?}", first.source_boundary),
    );
    attributes.insert("payload.library".to_string(), first.library.clone());
    attributes.insert("payload.symbol".to_string(), first.symbol.clone());
    attributes
}

fn payload_evidence(segments: &[&PayloadSegment]) -> Vec<SemanticEvidence> {
    segments
        .iter()
        .map(|segment| SemanticEvidence {
            kind: SemanticEvidenceKind::PayloadSegment,
            id: segment.segment_id.get(),
            role: "llm.request.payload".to_string(),
        })
        .collect()
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
    text: String,
    json: Option<Value>,
    model: Option<String>,
}

fn parse_llm_request_body(body: &[u8]) -> Option<LlmRequestBody> {
    let text = String::from_utf8_lossy(body).into_owned();
    let json = serde_json::from_slice::<Value>(body).ok();
    if let Some(value) = &json
        && json_value_is_llm_request(value)
    {
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        return Some(LlmRequestBody { text, json, model });
    }
    if lossy_text_is_llm_request(&text) {
        Some(LlmRequestBody {
            model: extract_json_string_lossy(&text, "model"),
            text,
            json: None,
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
        .get("llm.request.model")
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

fn payload_operation_ids(segments: &[&PayloadSegment]) -> String {
    let mut ids = BTreeSet::new();
    for segment in segments {
        ids.insert(segment.operation_id);
    }
    ids.into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

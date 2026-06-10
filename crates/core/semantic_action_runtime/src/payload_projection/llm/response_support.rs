//! Shared helpers for LLM response projection.

use std::collections::{BTreeMap, BTreeSet};

use model_core::payload::{
    PayloadOperationCompletionState, PayloadSegment, PayloadSourceBoundary, PayloadTruncationState,
};
use semantic_action::{
    SemanticActionCompleteness, SemanticActionStatus, SemanticEvidence, SemanticEvidenceKind,
};

use crate::payload_projection::http::HttpResponseParts;

use super::body::{LlmResponseBody, TokenUsage};
use super::stream::PayloadStreamGroupKey;

pub(super) fn llm_response_attributes(
    segments: &[&PayloadSegment],
    raw_bytes: &[u8],
    http: &HttpResponseParts,
    body: &LlmResponseBody,
) -> BTreeMap<String, String> {
    let first = segments[0];
    let mut attributes = raw_llm_response_attributes(segments, &http.body, body);
    attributes.insert(
        "llm.response.raw_payload_bytes".to_string(),
        raw_bytes.len().to_string(),
    );
    attributes.insert("http.response.body_text".to_string(), body.text.clone());
    if body.stream {
        attributes.insert("http.response.body_format".to_string(), "sse".to_string());
        attributes.insert(
            "http.response.body_json_state".to_string(),
            "not_applicable_sse".to_string(),
        );
    } else if let Some(body_json) = &body.json {
        attributes.insert("http.response.body_format".to_string(), "json".to_string());
        attributes.insert("http.response.body_json".to_string(), body_json.to_string());
        attributes.insert(
            "http.response.body_json_state".to_string(),
            "valid".to_string(),
        );
    } else {
        attributes.insert("http.response.body_format".to_string(), "text".to_string());
        attributes.insert(
            "http.response.body_json_state".to_string(),
            "invalid_or_unavailable".to_string(),
        );
    }
    attributes.insert(
        "http.response.protocol".to_string(),
        http.protocol.to_string(),
    );
    attributes.insert("network.protocol.name".to_string(), "http".to_string());
    attributes.insert(
        "network.protocol.version".to_string(),
        http.protocol.to_string(),
    );
    attributes.insert("url.scheme".to_string(), http.scheme.to_string());
    if let Some(status_code) = &http.status_code {
        attributes.insert("http.response.status_code".to_string(), status_code.clone());
    }
    if let Some(reason) = &http.reason {
        attributes.insert("http.response.reason".to_string(), reason.clone());
    }
    if let Some(stream_id) = http.stream_id {
        attributes.insert("http.response.stream_id".to_string(), stream_id.to_string());
    }
    if let Some(headers) = &http.headers_text {
        attributes.insert("http.response.headers_text".to_string(), headers.clone());
        attributes.insert(
            "http.response.headers_encoding".to_string(),
            "text".to_string(),
        );
    }
    if let Some(headers) = &http.headers_hpack_base64 {
        attributes.insert(
            "http.response.headers_hpack_base64".to_string(),
            headers.clone(),
        );
        attributes.insert(
            "http.response.headers_encoding".to_string(),
            "hpack".to_string(),
        );
    }
    attributes.insert(
        "payload.stream_key".to_string(),
        first.stream_key.to_string(),
    );
    attributes
}

pub(super) fn raw_llm_response_attributes(
    segments: &[&PayloadSegment],
    body_bytes: &[u8],
    body: &LlmResponseBody,
) -> BTreeMap<String, String> {
    let first = segments[0];
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "llm.response.payload_bytes".to_string(),
        body_bytes.len().to_string(),
    );
    attributes.insert("llm.response.payload_text".to_string(), body.text.clone());
    attributes.insert("llm.response.stream".to_string(), body.stream.to_string());
    attributes.insert(
        "llm.response.body_format".to_string(),
        llm_response_body_format(body).to_string(),
    );
    attributes.insert("llm.response.done".to_string(), body.done.to_string());
    attributes.insert(
        "llm.response.chunk_count".to_string(),
        body.chunk_count.to_string(),
    );
    if let Some(output_text) = &body.output_text {
        attributes.insert("llm.response.output_text".to_string(), output_text.clone());
    }
    if let Some(content_text) = &body.content_text {
        attributes.insert(
            "llm.response.content_text".to_string(),
            content_text.clone(),
        );
    }
    if let Some(reasoning_text) = &body.reasoning_text {
        attributes.insert(
            "llm.response.reasoning_text".to_string(),
            reasoning_text.clone(),
        );
    }
    if let Some(tool_calls_json) = &body.tool_calls_json {
        attributes.insert(
            "llm.response.tool_calls_json".to_string(),
            tool_calls_json.clone(),
        );
    }
    if let Some(model) = body.model.as_deref() {
        attributes.insert("llm.response.model".to_string(), model.to_string());
    }
    if let Some(usage) = &body.token_usage {
        insert_token_usage_attributes(&mut attributes, usage);
    }
    attributes.insert(
        "payload.stream_key".to_string(),
        first.stream_key.to_string(),
    );
    attributes.insert(
        "payload.operation_id".to_string(),
        first.operation_id.to_string(),
    );
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

fn insert_token_usage_attributes(attributes: &mut BTreeMap<String, String>, usage: &TokenUsage) {
    insert_token_count(
        attributes,
        "llm.response.prompt_tokens",
        usage.prompt_tokens,
    );
    insert_token_count(
        attributes,
        "llm.response.completion_tokens",
        usage.completion_tokens,
    );
    insert_token_count(attributes, "llm.response.total_tokens", usage.total_tokens);
    insert_token_count(
        attributes,
        "llm.response.cached_prompt_tokens",
        usage.cached_prompt_tokens,
    );
    insert_token_count(
        attributes,
        "llm.response.reasoning_tokens",
        usage.reasoning_tokens,
    );
    insert_token_count(
        attributes,
        "llm.response.prompt_cache_hit_tokens",
        usage.prompt_cache_hit_tokens,
    );
    insert_token_count(
        attributes,
        "llm.response.prompt_cache_miss_tokens",
        usage.prompt_cache_miss_tokens,
    );
}

fn insert_token_count(
    attributes: &mut BTreeMap<String, String>,
    key: &'static str,
    count: Option<u64>,
) {
    if let Some(count) = count {
        attributes.insert(key.to_string(), count.to_string());
    }
}

fn llm_response_body_format(body: &LlmResponseBody) -> &'static str {
    if body.stream {
        "sse"
    } else if body.json.is_some() {
        "json"
    } else {
        "text"
    }
}

pub(super) fn payload_evidence(segments: &[&PayloadSegment]) -> Vec<SemanticEvidence> {
    segments
        .iter()
        .map(|segment| SemanticEvidence {
            kind: SemanticEvidenceKind::PayloadSegment,
            id: segment.segment_id.get(),
            role: "llm.response.payload".to_string(),
        })
        .collect()
}

pub(super) fn plaintext_transport_scheme(source_boundary: PayloadSourceBoundary) -> &'static str {
    match source_boundary {
        PayloadSourceBoundary::TlsUserSpace => "https",
        PayloadSourceBoundary::Syscall => "http",
        PayloadSourceBoundary::Stdio => {
            unreachable!("stdio payloads are not HTTP transport candidates")
        }
    }
}

pub(super) fn llm_response_status(
    segments: &[&PayloadSegment],
    http_complete: bool,
    body: &LlmResponseBody,
) -> SemanticActionStatus {
    if segments.iter().any(|segment| {
        segment.operation_completion_state == PayloadOperationCompletionState::Failed
    }) {
        SemanticActionStatus::Error
    } else if body.done
        || http_complete
        || (!body.stream && operation_segments_are_complete(segments))
    {
        SemanticActionStatus::Success
    } else {
        SemanticActionStatus::InProgress
    }
}

pub(super) fn llm_response_completeness(
    segments: &[&PayloadSegment],
    http_complete: bool,
    body: &LlmResponseBody,
) -> SemanticActionCompleteness {
    if matches!(
        llm_response_status(segments, http_complete, body),
        SemanticActionStatus::Success
    ) && segments
        .iter()
        .all(|segment| segment_capture_is_complete(segment))
    {
        SemanticActionCompleteness::Complete
    } else {
        SemanticActionCompleteness::Partial
    }
}

pub(super) fn llm_response_title(attributes: &BTreeMap<String, String>) -> String {
    attributes
        .get("llm.response.model")
        .map(|model| format!("LLM response {}", model))
        .unwrap_or_else(|| "LLM response".to_string())
}

pub(super) fn llm_stream_action_id(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    segment: &PayloadSegment,
) -> String {
    format!(
        "trace:{}:payload-stream:{}:{}:{}:llm.response",
        segment.trace_id.get(),
        key.process.pid,
        key.stream_key,
        message_start
    )
}

pub(super) fn llm_raw_stream_action_id(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    segment: &PayloadSegment,
) -> String {
    format!(
        "trace:{}:payload-stream:{}:{}:{}:raw:llm.response",
        segment.trace_id.get(),
        key.process.pid,
        key.stream_key,
        message_start
    )
}

fn segment_capture_is_complete(segment: &PayloadSegment) -> bool {
    segment.truncation == PayloadTruncationState::Complete
        && segment.operation_completion_state == PayloadOperationCompletionState::Success
        && segment.operation_original_size == segment.operation_captured_size
}

fn operation_segments_are_complete(segments: &[&PayloadSegment]) -> bool {
    let Some(first) = segments.first() else {
        return false;
    };
    let mut expected_offset = 0_u64;
    for segment in segments {
        if segment.operation_offset != expected_offset {
            return false;
        }
        expected_offset = expected_offset.saturating_add(segment.captured_size);
    }
    expected_offset == first.operation_captured_size
        && first.operation_captured_size == first.operation_original_size
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

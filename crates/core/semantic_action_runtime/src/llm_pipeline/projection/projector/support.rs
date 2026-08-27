//! Shared helpers for LLM response projection.

use std::collections::BTreeMap;

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::{PayloadSegment, PayloadSourceBoundary};
use semantic_action::{
    LlmTokenUsage, SemanticActionCompleteness, SemanticActionStatus, SemanticEvidence,
    attr_keys as attrs, evidence_roles, validated_model_identifier,
};

use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::stream::response::{LlmResponseBody, ProviderStreamUpdate};
use crate::llm_pipeline::transport::HttpResponseParts;
use crate::llm_pipeline::transport::evidence::EvidenceSnapshot;

use super::super::retention::{insert_payload_snapshot_attributes, payload_snapshot_evidence};

pub(super) fn http_response_can_evict(http: &HttpResponseParts) -> bool {
    !http.body_boundary_known || http.complete
}

pub(super) fn llm_response_attributes(
    config: &SemanticRetentionConfig,
    evidence: &EvidenceSnapshot,
    raw_bytes: &[u8],
    http: &HttpResponseParts,
    body: &LlmResponseBody,
) -> BTreeMap<String, String> {
    let first = evidence
        .first
        .as_ref()
        .expect("response projection requires first evidence");
    let mut attributes = raw_llm_response_attributes(config, evidence, &http.body, body);
    attributes.insert(
        attrs::llm_response::RAW_PAYLOAD_BYTES.to_string(),
        raw_bytes.len().to_string(),
    );
    if body.stream {
        attributes.insert(
            attrs::http_response::BODY_FORMAT.to_string(),
            "sse".to_string(),
        );
        attributes.insert(
            attrs::http_response::BODY_JSON_STATE.to_string(),
            "not_applicable_sse".to_string(),
        );
    } else if body.json_valid {
        attributes.insert(
            attrs::http_response::BODY_FORMAT.to_string(),
            "json".to_string(),
        );
        attributes.insert(
            attrs::http_response::BODY_JSON_STATE.to_string(),
            "valid".to_string(),
        );
    } else {
        attributes.insert(
            attrs::http_response::BODY_FORMAT.to_string(),
            "text".to_string(),
        );
        attributes.insert(
            attrs::http_response::BODY_JSON_STATE.to_string(),
            "invalid_or_unavailable".to_string(),
        );
    }
    attributes.insert(
        attrs::http_response::PROTOCOL.to_string(),
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
    if let Some(status_code) = &http.status_code {
        attributes.insert(
            attrs::http_response::STATUS_CODE.to_string(),
            status_code.clone(),
        );
    }
    if let Some(reason) = &http.reason {
        attributes.insert(attrs::http_response::REASON.to_string(), reason.clone());
    }
    if let Some(stream_id) = http.stream_id {
        attributes.insert(
            attrs::http_response::STREAM_ID.to_string(),
            stream_id.to_string(),
        );
    }
    if let Some(headers) = &http.headers_text {
        attributes.insert(
            attrs::http_response::HEADERS_TEXT.to_string(),
            headers.clone(),
        );
        attributes.insert(
            attrs::http_response::HEADERS_ENCODING.to_string(),
            "text".to_string(),
        );
    }
    if let Some(headers) = &http.headers_hpack_base64 {
        attributes.insert(
            attrs::http_response::HEADERS_HPACK_BASE64.to_string(),
            headers.clone(),
        );
        attributes.insert(
            attrs::http_response::HEADERS_ENCODING.to_string(),
            "hpack".to_string(),
        );
    }
    attributes.insert(
        attrs::payload::STREAM_KEY.to_string(),
        first.stream_key.to_string(),
    );
    attributes
}

pub(super) fn raw_llm_response_attributes(
    config: &SemanticRetentionConfig,
    evidence: &EvidenceSnapshot,
    body_bytes: &[u8],
    body: &LlmResponseBody,
) -> BTreeMap<String, String> {
    let first = evidence
        .first
        .as_ref()
        .expect("response projection requires first evidence");
    let mut attributes = BTreeMap::new();
    attributes.insert(
        attrs::llm_response::PAYLOAD_BYTES.to_string(),
        body_bytes.len().to_string(),
    );
    attributes.insert(
        attrs::llm_response::STREAM.to_string(),
        body.stream.to_string(),
    );
    attributes.insert(
        attrs::llm_response::BODY_FORMAT.to_string(),
        llm_response_body_format(body).to_string(),
    );
    attributes.insert(attrs::llm_response::DONE.to_string(), body.done.to_string());
    attributes.insert(
        attrs::llm_response::CHUNK_COUNT.to_string(),
        body.chunk_count.to_string(),
    );
    if let Some(model) = body.model.as_deref().and_then(validated_model_identifier) {
        attributes.insert(attrs::llm_response::MODEL.to_string(), model.to_string());
    }
    attributes.insert(
        attrs::llm_response::PROVIDER_ID.to_string(),
        body.provider_id.to_string(),
    );
    if config.llm_response_assembled_provider_enabled() {
        if let Some(content_text) = body.content_text.as_deref() {
            attributes.insert(
                attrs::llm_response::CONTENT_TEXT.to_string(),
                content_text.to_string(),
            );
        }
        if let Some(reasoning_text) = body.reasoning_text.as_deref() {
            attributes.insert(
                attrs::llm_response::REASONING_TEXT.to_string(),
                reasoning_text.to_string(),
            );
        }
    }
    if config.llm_response_tool_calls_enabled()
        && let Some(tool_calls_json) = body.tool_calls_json.as_deref()
    {
        attributes.insert(
            attrs::llm_response::TOOL_CALLS_JSON.to_string(),
            tool_calls_json.to_string(),
        );
    }
    if config.llm_response_usage_enabled()
        && let Some(usage) = &body.token_usage
    {
        insert_token_usage_attributes(&mut attributes, usage);
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
    insert_payload_snapshot_attributes(&mut attributes, evidence);
    attributes.insert(
        attrs::payload::SOURCE_BOUNDARY.to_string(),
        format!("{:?}", first.source_boundary),
    );
    attributes.insert(attrs::payload::LIBRARY.to_string(), first.library.clone());
    attributes.insert(attrs::payload::SYMBOL.to_string(), first.symbol.clone());
    attributes
}

fn insert_token_usage_attributes(attributes: &mut BTreeMap<String, String>, usage: &LlmTokenUsage) {
    insert_token_count(
        attributes,
        attrs::llm_response::PROMPT_TOKENS,
        usage.prompt_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::COMPLETION_TOKENS,
        usage.completion_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::TOTAL_TOKENS,
        usage.total_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::CACHED_PROMPT_TOKENS,
        usage.cached_prompt_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::REASONING_TOKENS,
        usage.reasoning_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::PROMPT_CACHE_HIT_TOKENS,
        usage.prompt_cache_hit_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::PROMPT_CACHE_MISS_TOKENS,
        usage.prompt_cache_miss_tokens,
    );
    insert_token_count(
        attributes,
        attrs::llm_response::CACHE_CREATION_TOKENS,
        usage.cache_creation_tokens,
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
    } else if body.json_valid {
        "json"
    } else {
        "text"
    }
}

pub(super) fn payload_evidence(evidence: &EvidenceSnapshot) -> Vec<SemanticEvidence> {
    payload_snapshot_evidence(evidence, evidence_roles::llm_response::PAYLOAD)
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
    evidence: &EvidenceSnapshot,
    http_complete: bool,
    body: &LlmResponseBody,
) -> SemanticActionStatus {
    llm_response_status_from_progress(
        evidence,
        http_complete,
        &ProviderStreamUpdate {
            done: body.done,
            stream: body.stream,
            chunk_count: body.chunk_count,
        },
    )
}

pub(super) fn llm_response_status_from_progress(
    evidence: &EvidenceSnapshot,
    http_complete: bool,
    progress: &ProviderStreamUpdate,
) -> SemanticActionStatus {
    if evidence.any_operation_failed {
        SemanticActionStatus::Error
    } else if progress.done
        || http_complete
        || (!progress.stream && evidence.operation_segments_are_complete())
    {
        SemanticActionStatus::Success
    } else {
        SemanticActionStatus::InProgress
    }
}

pub(super) fn llm_response_completeness(
    evidence: &EvidenceSnapshot,
    http_complete: bool,
    body: &LlmResponseBody,
) -> SemanticActionCompleteness {
    if matches!(
        llm_response_status(evidence, http_complete, body),
        SemanticActionStatus::Success
    ) && evidence.all_capture_complete
    {
        SemanticActionCompleteness::Complete
    } else {
        SemanticActionCompleteness::Partial
    }
}

pub(super) fn llm_response_title(attributes: &BTreeMap<String, String>) -> String {
    attributes
        .get(attrs::llm_response::MODEL)
        .map(|model| format!("LLM response {}", model))
        .unwrap_or_else(|| "LLM response".to_string())
}

pub(super) fn llm_stream_action_id(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    segment: &PayloadSegment,
    http_stream_id: Option<u32>,
) -> String {
    match http_stream_id {
        Some(stream_id) => format!(
            "trace:{}:payload-stream:{}:{}:{}:h2:{}:llm.response",
            segment.trace_id.get(),
            key.process.get(),
            key.stream_key,
            message_start,
            stream_id
        ),
        None => format!(
            "trace:{}:payload-stream:{}:{}:{}:llm.response",
            segment.trace_id.get(),
            key.process.get(),
            key.stream_key,
            message_start
        ),
    }
}

pub(super) fn llm_raw_stream_action_id(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    segment: &PayloadSegment,
) -> String {
    format!(
        "trace:{}:payload-stream:{}:{}:{}:raw:llm.response",
        segment.trace_id.get(),
        key.process.get(),
        key.stream_key,
        message_start
    )
}

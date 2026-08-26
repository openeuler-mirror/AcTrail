//! LLM request projection from split HTTP payloads.

use std::collections::BTreeMap;

use config_core::daemon::{LlmRequestContentRetention, SemanticRetentionConfig};
use model_core::payload::{
    PayloadOperationCompletionState, PayloadSegment, PayloadSourceBoundary, PayloadTruncationState,
};
use semantic_action::{
    LlmRequestContentWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionStatus, attr_keys as attrs, evidence_roles, validated_model_identifier,
};
use serde_json::Value;

use crate::llm_pipeline::transport::HttpRequestParts;

use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::provider::{LlmRequestParserInput, parse_json_request};

use super::super::retention::semantic_payload_draft;
use super::super::retention::{
    FORMAT_VERSION, TrajectoryHistoryProjection, canonical_request_content,
    canonical_shape_metadata,
};
use super::super::retention::{insert_payload_span_attributes, payload_aggregate_evidence};

mod tool_results;

pub(crate) use tool_results::ProjectedLlmToolResult;
use tool_results::project_tool_results;

pub(crate) struct ProjectedLlmRequestAction {
    pub(crate) action: SemanticAction,
    pub(crate) content: Option<LlmRequestContentWrite>,
    pub(crate) trajectory_history: Option<ProjectedLlmRequestHistory>,
    pub(crate) tool_results: Vec<ProjectedLlmToolResult>,
    pub(crate) payload_segments: Vec<PayloadSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectedLlmRequestHistory {
    pub(crate) action_id: String,
    pub(crate) classifier_id: String,
    pub(crate) provider_context: ProviderContextReference,
    pub(crate) history: TrajectoryHistoryProjection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProviderContextReference {
    NotProvider,
    Root,
    PreviousResponse(String),
}

pub(super) fn project_stream_llm_request_action(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpRequestParts,
    segments: &[&PayloadSegment],
) -> Option<ProjectedLlmRequestAction> {
    let body = parse_llm_request_body(&http, codecs)?;
    let first = *segments.first()?;
    let action_id = llm_stream_action_id(key, message_start, first, http.stream_id);
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let content_projection = project_request_content(config, first.trace_id, &action_id, &body)
        .expect("projecting canonical LLM request content should not fail for parsed JSON");
    let attributes = llm_attributes(
        config,
        segments,
        raw_bytes,
        &http,
        &body,
        content_projection.metadata.as_ref(),
    );
    let evidence = payload_aggregate_evidence(segments, evidence_roles::llm_request::PAYLOAD);
    let tool_results = body.json.as_ref().map_or_else(Vec::new, |value| {
        project_tool_results(
            config,
            first.trace_id,
            &first.process,
            first.observed_at,
            &action_id,
            value,
        )
    });
    let payload_segments =
        if config.l4_payload.enabled || !config.l0_llm_call.retain_assembled_payload() {
            Vec::new()
        } else {
            vec![semantic_payload_draft(first, raw_bytes)]
        };
    Some(ProjectedLlmRequestAction {
        trajectory_history: content_projection.trajectory_history.map(|history| {
            ProjectedLlmRequestHistory {
                action_id: action_id.clone(),
                classifier_id: body.classifier_id.clone(),
                provider_context: body
                    .json
                    .as_ref()
                    .map(|value| {
                        provider_context_reference(
                            value,
                            config.l0_llm_call.trajectory.max_structural_bytes_per_atom as usize,
                        )
                    })
                    .unwrap_or(ProviderContextReference::NotProvider),
                history,
            }
        }),
        action: SemanticAction {
            action_id,
            trace_id: first.trace_id,
            kind: SemanticActionKind::LlmRequest,
            title: llm_title(&attributes),
            start_time: first.observed_at,
            end_time: segments.last().map(|segment| segment.observed_at),
            process: first.process.clone(),
            status: llm_status(segments),
            completeness: llm_stream_completeness(segments),
            attributes,
            evidence,
        },
        content: content_projection.content,
        tool_results,
        payload_segments,
    })
}

struct RequestContentProjection {
    content: Option<LlmRequestContentWrite>,
    metadata: Option<RequestContentMetadata>,
    trajectory_history: Option<TrajectoryHistoryProjection>,
}

enum CanonicalBodyExport {
    Exported(String),
    TooLarge,
}

impl CanonicalBodyExport {
    fn state_attribute_value(&self) -> &'static str {
        match self {
            Self::Exported(_) => "exported",
            Self::TooLarge => "too_large",
        }
    }

    fn body_json(&self) -> Option<&str> {
        match self {
            Self::Exported(body_json) => Some(body_json),
            Self::TooLarge => None,
        }
    }
}

struct CanonicalBodyMetadata {
    hash: String,
    bytes: u64,
    export: Option<CanonicalBodyExport>,
}

struct RequestContentMetadata {
    state: &'static str,
    format_version: Option<u32>,
    canonical_body: Option<CanonicalBodyMetadata>,
    block_count: Option<usize>,
    message_preview: Option<String>,
    user_message_count: Option<usize>,
    tool_result_count: Option<usize>,
    latest_user_message_hash: Option<String>,
    background_kind: Option<&'static str>,
}

fn project_request_content(
    config: &SemanticRetentionConfig,
    trace_id: model_core::ids::TraceId,
    action_id: &str,
    body: &LlmRequestBody,
) -> Result<RequestContentProjection, String> {
    if !config.llm_layer_enabled() {
        return Ok(RequestContentProjection {
            content: None,
            metadata: None,
            trajectory_history: None,
        });
    }
    match config.l0_llm_call.request_content {
        LlmRequestContentRetention::None => Ok(RequestContentProjection {
            content: None,
            trajectory_history: None,
            metadata: Some(RequestContentMetadata {
                state: "none",
                format_version: None,
                canonical_body: None,
                block_count: None,
                message_preview: None,
                user_message_count: None,
                tool_result_count: None,
                latest_user_message_hash: None,
                background_kind: None,
            }),
        }),
        LlmRequestContentRetention::Shape => Ok(shape_projection(body)),
        LlmRequestContentRetention::CanonicalBlocks => {
            let Some(value) = body.json.as_ref() else {
                return Ok(shape_projection(body));
            };
            let content = canonical_request_content(
                trace_id,
                action_id,
                value,
                config.llm_trajectory_enabled(),
            )?;
            let canonical_body_export = config.llm_request_body_export_enabled().then(|| {
                if content.canonical_body.bytes <= config.l0_llm_call.request_body_export_max_bytes
                {
                    CanonicalBodyExport::Exported(content.canonical_body.json.clone())
                } else {
                    CanonicalBodyExport::TooLarge
                }
            });
            Ok(RequestContentProjection {
                metadata: Some(RequestContentMetadata {
                    state: "canonical_blocks",
                    format_version: Some(FORMAT_VERSION),
                    canonical_body: Some(CanonicalBodyMetadata {
                        hash: content.canonical_body.hash.clone(),
                        bytes: content.canonical_body.bytes,
                        export: canonical_body_export,
                    }),
                    block_count: Some(content.block_count),
                    message_preview: content.message_preview.clone(),
                    user_message_count: Some(content.user_message_count),
                    tool_result_count: Some(content.tool_result_count),
                    latest_user_message_hash: content.latest_user_message_hash.clone(),
                    background_kind: content.background_kind,
                }),
                content: Some(content.write),
                trajectory_history: content.trajectory_history,
            })
        }
    }
}

fn shape_projection(body: &LlmRequestBody) -> RequestContentProjection {
    let (canonical_body, message_preview, user_messages, tool_result_count, background_kind) = body
        .json
        .as_ref()
        .map_or((None, None, None, None, None), |value| {
            let (hash, bytes, preview, user_messages, tool_result_count, background_kind) =
                canonical_shape_metadata(value);
            (
                Some(CanonicalBodyMetadata {
                    hash,
                    bytes,
                    export: None,
                }),
                preview,
                Some(user_messages),
                Some(tool_result_count),
                background_kind,
            )
        });
    RequestContentProjection {
        content: None,
        trajectory_history: None,
        metadata: Some(RequestContentMetadata {
            state: "shape",
            format_version: body.json.as_ref().map(|_| FORMAT_VERSION),
            canonical_body,
            block_count: None,
            message_preview,
            user_message_count: user_messages.as_ref().map(|metadata| metadata.count),
            tool_result_count,
            latest_user_message_hash: user_messages.and_then(|metadata| metadata.latest_hash),
            background_kind,
        }),
    }
}

fn llm_attributes(
    config: &SemanticRetentionConfig,
    segments: &[&PayloadSegment],
    raw_bytes: &[u8],
    http: &HttpRequestParts,
    body: &LlmRequestBody,
    content: Option<&RequestContentMetadata>,
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
    if let Some(model) = body.model.as_deref().and_then(validated_model_identifier) {
        attributes.insert(attrs::llm_request::MODEL.to_string(), model.to_string());
    }
    attributes.insert(
        attrs::llm_request::CLASSIFIER_ID.to_string(),
        body.classifier_id.to_string(),
    );
    if let Some(protocol_id) = &body.protocol_id {
        attributes.insert(
            attrs::llm_request::PROTOCOL_ID.to_string(),
            protocol_id.to_string(),
        );
    }
    if let Some(content) = content {
        attributes.insert(
            attrs::llm_request::CONTENT_STATE.to_string(),
            content.state.to_string(),
        );
        if let Some(format_version) = content.format_version {
            attributes.insert(
                attrs::llm_request::CONTENT_FORMAT_VERSION.to_string(),
                format_version.to_string(),
            );
        }
        if let Some(canonical_body) = content.canonical_body.as_ref() {
            if let Some(export) = canonical_body.export.as_ref() {
                if let Some(body_json) = export.body_json() {
                    attributes.insert(
                        attrs::llm_request::CANONICAL_BODY_JSON.to_string(),
                        body_json.to_string(),
                    );
                }
                attributes.insert(
                    attrs::llm_request::CANONICAL_BODY_EXPORT_STATE.to_string(),
                    export.state_attribute_value().to_string(),
                );
            }
            attributes.insert(
                attrs::llm_request::CANONICAL_BODY_HASH.to_string(),
                canonical_body.hash.clone(),
            );
            attributes.insert(
                attrs::llm_request::CANONICAL_BODY_BYTES.to_string(),
                canonical_body.bytes.to_string(),
            );
        }
        if let Some(block_count) = content.block_count {
            attributes.insert(
                attrs::llm_request::BLOCK_COUNT.to_string(),
                block_count.to_string(),
            );
        }
        if let Some(preview) = content.message_preview.as_deref() {
            attributes.insert(
                attrs::llm_request::MESSAGE_PREVIEW.to_string(),
                preview.to_string(),
            );
        }
        if let Some(user_message_count) = content.user_message_count {
            attributes.insert(
                attrs::llm_request::USER_MESSAGE_COUNT.to_string(),
                user_message_count.to_string(),
            );
        }
        if let Some(tool_result_count) = content.tool_result_count {
            attributes.insert(
                attrs::llm_request::TOOL_RESULT_COUNT.to_string(),
                tool_result_count.to_string(),
            );
        }
        if let Some(hash) = content.latest_user_message_hash.as_deref() {
            attributes.insert(
                attrs::llm_request::LATEST_USER_MESSAGE_HASH.to_string(),
                hash.to_string(),
            );
        }
        if let Some(background_kind) = content.background_kind {
            attributes.insert(
                attrs::llm_request::BACKGROUND_KIND.to_string(),
                background_kind.to_string(),
            );
        }
    } else if config.llm_layer_enabled() {
        attributes.insert(
            attrs::llm_request::CONTENT_STATE.to_string(),
            "unavailable".to_string(),
        );
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
    classifier_id: String,
    protocol_id: Option<String>,
    model: Option<String>,
    json: Option<Value>,
}

fn parse_llm_request_body(
    http: &HttpRequestParts,
    codecs: &LlmCodecRegistry,
) -> Option<LlmRequestBody> {
    LlmRequestBodyParser { codecs }.parse(http)
}

struct LlmRequestBodyParser<'a> {
    codecs: &'a LlmCodecRegistry,
}

impl LlmRequestBodyParser<'_> {
    fn parse(&self, http: &HttpRequestParts) -> Option<LlmRequestBody> {
        let body = &http.body;
        if let Some(decoded) = self.codecs.decode_request(http)
            && let Ok(value) = serde_json::from_slice::<Value>(&decoded.body)
        {
            let input = LlmRequestParserInput { json: &value };
            let parsed = parse_json_request(&input);
            let classifier_id = decoded.classifier_id.or_else(|| {
                parsed
                    .as_ref()
                    .map(|parsed| parsed.classifier_id.to_string())
            })?;
            return Some(LlmRequestBody {
                json_valid: true,
                classifier_id,
                protocol_id: decoded.protocol_id.or_else(|| {
                    parsed
                        .as_ref()
                        .and_then(|parsed| parsed.protocol_id.map(ToString::to_string))
                }),
                model: valid_model(decoded.model)
                    .or_else(|| parsed.and_then(|parsed| valid_model(parsed.model))),
                json: Some(value),
            });
        }
        if let Ok(value) = serde_json::from_slice::<Value>(body) {
            let input = LlmRequestParserInput { json: &value };
            let parsed = parse_json_request(&input)?;
            return Some(LlmRequestBody {
                json_valid: true,
                classifier_id: parsed.classifier_id.to_string(),
                protocol_id: parsed.protocol_id.map(ToString::to_string),
                model: valid_model(parsed.model),
                json: Some(value),
            });
        }
        let text = String::from_utf8_lossy(body);
        let model = extract_json_string_lossy(&text, "model");
        if model.is_some() && lossy_text_has_llm_shape(&text) {
            Some(LlmRequestBody {
                json_valid: false,
                classifier_id: "generic-json-request".to_string(),
                protocol_id: None,
                model,
                json: None,
            })
        } else {
            None
        }
    }
}

fn provider_context_reference(value: &Value, maximum_bytes: usize) -> ProviderContextReference {
    match value.get("previous_response_id") {
        None => ProviderContextReference::NotProvider,
        Some(Value::Null) => ProviderContextReference::Root,
        Some(Value::String(response_id))
            if !response_id.is_empty() && response_id.len() <= maximum_bytes =>
        {
            ProviderContextReference::PreviousResponse(response_id.clone())
        }
        Some(_) => ProviderContextReference::Root,
    }
}

fn lossy_text_has_llm_shape(text: &str) -> bool {
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
    let value = after_key[colon + 1..].trim_start_matches([' ', '\t', '\r', '\n', '\0']);
    let quoted = value.strip_prefix('"')?;
    let end = quoted.find('"')?;
    validated_model_identifier(&quoted[..end]).map(ToOwned::to_owned)
}

fn valid_model(model: Option<String>) -> Option<String> {
    model.and_then(|value| validated_model_identifier(&value).map(ToOwned::to_owned))
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
    http_stream_id: Option<u32>,
) -> String {
    match http_stream_id {
        Some(stream_id) => format!(
            "trace:{}:payload-stream:{}:{}:{}:h2:{}:llm.request",
            segment.trace_id.get(),
            key.process.get(),
            key.stream_key,
            message_start,
            stream_id
        ),
        None => format!(
            "trace:{}:payload-stream:{}:{}:{}:llm.request",
            segment.trace_id.get(),
            key.process.get(),
            key.stream_key,
            message_start
        ),
    }
}

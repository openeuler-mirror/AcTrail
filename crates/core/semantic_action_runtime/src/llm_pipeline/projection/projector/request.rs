//! LLM request projection from split HTTP payloads.

use std::collections::BTreeMap;

use config_core::daemon::{LlmRequestContentRetention, SemanticRetentionConfig};
use model_core::payload::{
    PayloadCaptureState, PayloadOperationCompletionState, PayloadSegment, PayloadSourceBoundary,
};
use semantic_action::{
    LlmRequestContentWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionStatus, attr_keys as attrs, evidence_roles, validated_model_identifier,
};
use serde_json::Value;

use crate::llm_pipeline::transport::HttpRequestParts;

use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::provider::{LlmRequestFacts, LlmRequestParserInput, parse_json_request};

use super::super::retention::semantic_payload_draft;
use super::super::retention::{BackgroundRequestKind, TrajectoryHistoryProjection};
use super::super::retention::{insert_payload_span_attributes, payload_aggregate_evidence};

mod content;
mod selective;
mod strict;
mod tool_results;

use content::project_request_content;

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
    mut http: HttpRequestParts<'_>,
    segments: &[&PayloadSegment],
) -> Option<ProjectedLlmRequestAction> {
    let completeness = llm_stream_completeness(http.complete, segments);
    let body = if completeness == SemanticActionCompleteness::CaptureLimited {
        CaptureLimitedHttpRequestClassifier::classify(&http)
    } else {
        parse_llm_request_body(&http, codecs, config)
    }?;
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
        content_projection.metadata,
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
            completeness,
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
}

struct RequestContentMetadata {
    state: &'static str,
    format_version: Option<u32>,
    body_export: Option<CanonicalBodyExport>,
    block_count: Option<usize>,
    message_preview: Option<String>,
    user_message_count: Option<usize>,
    tool_result_count: Option<usize>,
    background_kind: Option<&'static str>,
}

fn llm_attributes(
    config: &SemanticRetentionConfig,
    segments: &[&PayloadSegment],
    raw_bytes: &[u8],
    http: &HttpRequestParts<'_>,
    body: &LlmRequestBody,
    content: Option<RequestContentMetadata>,
) -> BTreeMap<String, String> {
    let first = segments[0];
    let mut attributes = BTreeMap::new();
    attributes.insert(
        attrs::llm_request::RAW_PAYLOAD_BYTES.to_string(),
        raw_bytes.len().to_string(),
    );
    attributes.insert(
        attrs::llm_request::PAYLOAD_BYTES.to_string(),
        body.payload_bytes.to_string(),
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
        if let Some(export) = content.body_export {
            attributes.insert(
                attrs::llm_request::CANONICAL_BODY_EXPORT_STATE.to_string(),
                export.state_attribute_value().to_string(),
            );
            if let CanonicalBodyExport::Exported(body_json) = export {
                attributes.insert(
                    attrs::llm_request::CANONICAL_BODY_JSON.to_string(),
                    body_json,
                );
            }
        }
        if let Some(block_count) = content.block_count {
            attributes.insert(
                attrs::llm_request::BLOCK_COUNT.to_string(),
                block_count.to_string(),
            );
        }
        if let Some(preview) = content.message_preview {
            attributes.insert(attrs::llm_request::MESSAGE_PREVIEW.to_string(), preview);
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
    payload_bytes: usize,
    json_valid: bool,
    classifier_id: String,
    protocol_id: Option<String>,
    model: Option<String>,
    json: Option<Value>,
    content_available: bool,
    background_kind: Option<&'static str>,
}

fn parse_llm_request_body(
    http: &HttpRequestParts<'_>,
    codecs: &LlmCodecRegistry,
    config: &SemanticRetentionConfig,
) -> Option<LlmRequestBody> {
    LlmRequestBodyParser {
        codecs,
        retain_json: config.l0_llm_call.request_content != LlmRequestContentRetention::None
            || config.l0_llm_call.tool_results_enabled
            || config.llm_request_body_export_enabled()
            || config.llm_trajectory_enabled(),
    }
    .parse(http)
}

struct LlmRequestBodyParser<'a> {
    codecs: &'a LlmCodecRegistry,
    retain_json: bool,
}

struct DecodedRequestJson {
    facts: LlmRequestFacts,
    json: Option<Value>,
    background_kind: Option<&'static str>,
}

impl LlmRequestBodyParser<'_> {
    fn parse_json(&self, bytes: &[u8]) -> serde_json::Result<DecodedRequestJson> {
        if self.retain_json {
            let json: Value = serde_json::from_slice(bytes)?;
            Ok(DecodedRequestJson {
                facts: LlmRequestFacts::from_json(&json),
                json: Some(json),
                background_kind: None,
            })
        } else {
            let selected = selective::SelectedRequest::parse(bytes)?;
            Ok(DecodedRequestJson {
                background_kind: BackgroundRequestKind::classify(
                    selected.system_parts.iter().map(|s| s.as_ref()),
                ),
                facts: selected.facts,
                json: None,
            })
        }
    }
    fn parse(&self, http: &HttpRequestParts<'_>) -> Option<LlmRequestBody> {
        let body = http.body;
        if let Some(decoded) = self.codecs.decode_request(http)
            && let Ok(value) = self.parse_json(&decoded.body)
        {
            let input = LlmRequestParserInput {
                facts: &value.facts,
            };
            let parsed = parse_json_request(&input);
            let classifier_id = decoded.classifier_id.or_else(|| {
                parsed
                    .as_ref()
                    .map(|parsed| parsed.classifier_id.to_string())
            })?;
            return Some(LlmRequestBody {
                payload_bytes: body.len(),
                json_valid: true,
                classifier_id,
                protocol_id: decoded.protocol_id.or_else(|| {
                    parsed
                        .as_ref()
                        .and_then(|parsed| parsed.protocol_id.map(ToString::to_string))
                }),
                model: valid_model(decoded.model)
                    .or_else(|| parsed.and_then(|parsed| valid_model(parsed.model))),
                json: value.json,
                background_kind: value.background_kind,
                content_available: true,
            });
        }
        if let Ok(value) = self.parse_json(body) {
            let input = LlmRequestParserInput {
                facts: &value.facts,
            };
            let parsed = parse_json_request(&input);
            let classifier_id =
                parsed
                    .as_ref()
                    .map(|parsed| parsed.classifier_id)
                    .or_else(|| {
                        (!self.retain_json)
                            .then(|| CaptureLimitedHttpRequestClassifier::route(http))
                            .flatten()
                    })?;
            return Some(LlmRequestBody {
                payload_bytes: body.len(),
                json_valid: true,
                classifier_id: classifier_id.to_string(),
                protocol_id: parsed
                    .as_ref()
                    .and_then(|parsed| parsed.protocol_id.map(ToString::to_string)),
                model: valid_model(parsed.and_then(|parsed| parsed.model)),
                json: value.json,
                background_kind: value.background_kind,
                content_available: true,
            });
        }
        let text = String::from_utf8_lossy(body);
        let model = extract_json_string_lossy(&text, "model");
        if model.is_some() && lossy_text_has_llm_shape(&text) {
            Some(LlmRequestBody {
                payload_bytes: body.len(),
                json_valid: false,
                classifier_id: "generic-json-request".to_string(),
                protocol_id: None,
                model,
                json: None,
                background_kind: None,
                content_available: true,
            })
        } else {
            None
        }
    }
}

struct CaptureLimitedHttpRequestClassifier;

impl CaptureLimitedHttpRequestClassifier {
    fn classify(http: &HttpRequestParts<'_>) -> Option<LlmRequestBody> {
        if http.complete {
            return None;
        }
        let classifier_id = Self::route(http)?;
        Some(LlmRequestBody {
            payload_bytes: http.declared_body_len.unwrap_or(http.body.len()),
            json_valid: false,
            classifier_id: classifier_id.to_string(),
            protocol_id: None,
            model: None,
            json: None,
            background_kind: None,
            content_available: false,
        })
    }

    fn route(http: &HttpRequestParts<'_>) -> Option<&'static str> {
        if !http
            .method
            .as_deref()
            .is_some_and(|method| method.eq_ignore_ascii_case("POST"))
            || !http
                .headers_text
                .as_deref()
                .is_some_and(Self::has_json_content_type)
        {
            return None;
        }
        let path = http.path.as_deref()?.split('?').next()?;
        let classifier_id = match path {
            "/chat/completions" | "/v1/chat/completions" | "/api/v2/chat/completions" => {
                "openai-compatible-route"
            }
            "/responses" | "/v1/responses" => "openai-responses-route",
            "/messages" | "/v1/messages" => "anthropic-messages-route",
            _ => return None,
        };
        Some(classifier_id)
    }

    fn has_json_content_type(headers: &str) -> bool {
        headers.split("\r\n").skip(1).any(|line| {
            let Some((name, value)) = line.split_once(':') else {
                return false;
            };
            name.eq_ignore_ascii_case("content-type")
                && value.split(';').next().is_some_and(|media_type| {
                    media_type.trim().eq_ignore_ascii_case("application/json")
                })
        })
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

fn llm_stream_completeness(
    http_complete: bool,
    segments: &[&PayloadSegment],
) -> SemanticActionCompleteness {
    if http_complete {
        SemanticActionCompleteness::Complete
    } else if segments_capture_is_policy_limited(segments) {
        SemanticActionCompleteness::CaptureLimited
    } else {
        SemanticActionCompleteness::Partial
    }
}

fn segments_capture_is_policy_limited(segments: &[&PayloadSegment]) -> bool {
    let mut policy_limited = false;
    for segment in segments {
        match segment.capture_state() {
            PayloadCaptureState::PolicyLimited => policy_limited = true,
            PayloadCaptureState::Incomplete => return false,
            PayloadCaptureState::Complete => {}
        }
    }
    policy_limited
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

//! LLM response projection from inbound plaintext payloads.

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus};

use crate::llm_pipeline::config::StreamClassifierConfig;
use crate::llm_pipeline::transport::HttpResponseParts;

use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::response::{
    IncrementalSseCache, SseBodySource, parse_llm_response_body_incremental,
    parse_llm_response_progress,
};
use crate::llm_pipeline::transport::evidence::EvidenceSnapshot;

use super::super::retention::semantic_payload_draft;
use super::support::{
    http_response_can_evict, llm_raw_stream_action_id, llm_response_attributes,
    llm_response_completeness, llm_response_status_from_progress, llm_response_title,
    llm_stream_action_id, payload_evidence, plaintext_transport_scheme,
    raw_llm_response_attributes,
};

/// Identity of one in-flight LLM response message held by a stream state.
///
/// Recorded when the incremental SSE cache is first seeded (first chunk of the
/// message) and cleared when the message terminates. Used only to materialize
/// the full response action on trace close; the terminal chunk materializes
/// through the normal projection path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InFlightResponse {
    pub(crate) message_start: usize,
}

pub(super) struct LlmResponseProjection {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) provider_response_ids: Vec<ProjectedProviderResponseId>,
    pub(super) payload_segments: Vec<PayloadSegment>,
    pub(super) in_flight: Option<InFlightResponse>,
    pub(super) encoded_len: usize,
    pub(super) terminal: bool,
}

pub(crate) struct ProjectedProviderResponseId {
    pub(crate) action_id: String,
    pub(crate) provider_response_id: String,
}

fn in_flight_projection(encoded_len: usize, message_start: usize) -> LlmResponseProjection {
    LlmResponseProjection {
        actions: Vec::new(),
        provider_response_ids: Vec::new(),
        payload_segments: Vec::new(),
        in_flight: Some(InFlightResponse { message_start }),
        encoded_len,
        terminal: false,
    }
}

pub(super) fn project_stream_llm_response_message_actions(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpResponseParts,
    evidence: &EvidenceSnapshot,
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LlmResponseProjection> {
    let classifier_config = StreamClassifierConfig::from_semantic_retention(config);
    let progress = parse_llm_response_progress(
        SseBodySource::SplitHttp,
        &http.body,
        codecs,
        classifier_config,
        sse_cache,
        http.complete || force_terminal,
    )?;
    let first = evidence.first.as_ref()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let status = llm_response_status_from_progress(evidence, http.complete, &progress);
    if !force_terminal && status == SemanticActionStatus::InProgress {
        return Some(in_flight_projection(http.encoded_len, message_start));
    }
    let mut body = parse_llm_response_body_incremental(
        SseBodySource::SplitHttp,
        &http.body,
        codecs,
        classifier_config,
        sse_cache,
    )?;
    let attributes = llm_response_attributes(config, evidence, raw_bytes, &http, &body);
    let semantic_evidence = payload_evidence(evidence);
    let payload_segments = semantic_response_payloads(config, first, raw_bytes);
    let response = SemanticAction {
        action_id: llm_stream_action_id(key, message_start, first, http.stream_id),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: evidence.last.as_ref().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status,
        completeness: llm_response_completeness(evidence, http.complete, &body),
        attributes,
        evidence: semantic_evidence,
    };
    let provider_response_ids =
        projected_provider_response_ids(&response, body.provider_response_id.take());
    Some(LlmResponseProjection {
        actions: vec![response],
        provider_response_ids,
        payload_segments,
        in_flight: None,
        encoded_len: http.encoded_len,
        terminal: http_response_can_evict(&http) && status != SemanticActionStatus::InProgress,
    })
}

pub(super) fn project_raw_stream_llm_response_actions(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    evidence: &EvidenceSnapshot,
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LlmResponseProjection> {
    let classifier_config = StreamClassifierConfig::from_semantic_retention(config);
    let progress = parse_llm_response_progress(
        SseBodySource::RawBytes,
        bytes,
        codecs,
        classifier_config,
        sse_cache,
        force_terminal,
    )?;
    let first = evidence.first.as_ref()?;
    let status = llm_response_status_from_progress(evidence, false, &progress);
    if !force_terminal && status == SemanticActionStatus::InProgress {
        return Some(in_flight_projection(bytes.len(), message_start));
    }
    let mut body = parse_llm_response_body_incremental(
        SseBodySource::RawBytes,
        bytes,
        codecs,
        classifier_config,
        sse_cache,
    )?;
    let attributes = raw_llm_response_attributes(config, evidence, bytes, &body);
    let payload_segments = semantic_response_payloads(config, first, bytes);
    let response = SemanticAction {
        action_id: llm_raw_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: evidence.last.as_ref().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status,
        completeness: llm_response_completeness(evidence, false, &body),
        attributes,
        evidence: payload_evidence(evidence),
    };
    let provider_response_ids =
        projected_provider_response_ids(&response, body.provider_response_id.take());
    Some(LlmResponseProjection {
        actions: vec![response],
        provider_response_ids,
        payload_segments,
        in_flight: None,
        encoded_len: bytes.len(),
        terminal: true,
    })
}

fn projected_provider_response_ids(
    response: &SemanticAction,
    provider_response_id: Option<String>,
) -> Vec<ProjectedProviderResponseId> {
    provider_response_id
        .map(|provider_response_id| ProjectedProviderResponseId {
            action_id: response.action_id.clone(),
            provider_response_id,
        })
        .into_iter()
        .collect()
}

fn semantic_response_payloads(
    config: &SemanticRetentionConfig,
    first: &PayloadSegment,
    assembled_bytes: &[u8],
) -> Vec<PayloadSegment> {
    if config.l4_payload.enabled || !config.l0_llm_call.retain_assembled_payload() {
        Vec::new()
    } else {
        vec![semantic_payload_draft(first, assembled_bytes)]
    }
}

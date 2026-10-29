//! LLM response projection from inbound plaintext payloads.

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{LlmResponseRetention, SemanticAction, SemanticActionKind};

use crate::llm_pipeline::config::StreamClassifierConfig;
use crate::llm_pipeline::projection::compact::LlmResponseObservation;
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
    llm_response_attributes, llm_response_completeness, llm_response_status, llm_response_title,
    payload_evidence, plaintext_transport_scheme,
};

/// Identifies an unfinished LLM response for completion or stream teardown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InFlightResponse {
    pub(crate) message_start: usize,
}

pub(super) struct LlmResponseProjection {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) response_observations: Vec<LlmResponseObservation>,
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
        response_observations: Vec::new(),
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
    _key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpResponseParts,
    evidence: &EvidenceSnapshot,
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
    body_decoding_failed: bool,
) -> Option<LlmResponseProjection> {
    let classifier_config = StreamClassifierConfig::from_semantic_retention(config);
    let retention = LlmResponseRetention {
        content: config.llm_response_assembled_provider_enabled(),
        tool_calls: config.llm_response_tool_calls_enabled(),
        usage: config.llm_response_usage_enabled(),
    };
    let mut body = if body_decoding_failed {
        // The decoder may no longer expose the consumed body. Finish the
        // existing parser without replacing its state with an empty body.
        sse_cache.as_mut()?.body()?
    } else {
        let progress = parse_llm_response_progress(
            SseBodySource::SplitHttp,
            &http.body,
            codecs,
            classifier_config,
            retention,
            sse_cache,
            http.complete || force_terminal,
        )?;
        // A finish reason does not stop consumption of trailing provider events.
        // Transport teardown asks the parser to finish with its available evidence.
        if !force_terminal && progress.termination.is_none() && !http.complete {
            return Some(in_flight_projection(http.encoded_len, message_start));
        }
        parse_llm_response_body_incremental(
            SseBodySource::SplitHttp,
            &http.body,
            codecs,
            classifier_config,
            retention,
            sse_cache,
        )?
    };
    let first = evidence.first.as_ref()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let status = llm_response_status(evidence, http.complete, &body);
    let terminal = body.termination.is_some() || http.complete;
    let attributes = llm_response_attributes(config, evidence, raw_bytes, &http, &body);
    let semantic_evidence = payload_evidence(evidence);
    let payload_segments = semantic_response_payloads(config, first, raw_bytes);
    let action_id = format!("{}:llm.response", http.message_id);
    let response = SemanticAction {
        action_id,
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
    let response_observations = if config.l0_llm_call.context_compact.enabled {
        vec![LlmResponseObservation {
            action_id: response.action_id.clone(),
            has_tool_calls: body.tool_calls_json.is_some(),
            provider_failed: body
                .termination
                .is_some_and(|termination| termination.is_failure()),
            content_text: body.content_text.take(),
        }]
    } else {
        Vec::new()
    };
    Some(LlmResponseProjection {
        actions: vec![response],
        response_observations,
        provider_response_ids,
        payload_segments,
        in_flight: (!terminal).then_some(InFlightResponse { message_start }),
        encoded_len: http.encoded_len,
        terminal,
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

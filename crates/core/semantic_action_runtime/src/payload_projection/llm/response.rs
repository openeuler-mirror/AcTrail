//! LLM response projection from inbound plaintext payloads.

use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind};

use crate::payload_projection::http::HttpResponseParts;

use super::body::parse_llm_response_body;
use super::response_support::{
    llm_raw_stream_action_id, llm_response_attributes, llm_response_completeness,
    llm_response_status, llm_response_title, llm_stream_action_id, payload_evidence,
    plaintext_transport_scheme, raw_llm_response_attributes,
};
use super::sse::sse_actions_for_response;
use super::stream::PayloadStreamGroupKey;

pub(super) fn project_stream_llm_response_message_actions(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpResponseParts,
    segments: &[&PayloadSegment],
) -> Option<Vec<SemanticAction>> {
    let body = parse_llm_response_body(&http.body)?;
    let first = *segments.first()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let attributes = llm_response_attributes(segments, raw_bytes, &http, &body);
    let evidence = payload_evidence(segments);
    let response = SemanticAction {
        action_id: llm_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status: llm_response_status(segments, http.complete, &body),
        completeness: llm_response_completeness(segments, http.complete, &body),
        confidence_millis: None,
        attributes,
        evidence,
    };
    let sse_actions = sse_actions_for_response(&response, &body, segments);
    let mut actions = vec![response];
    actions.extend(sse_actions);
    Some(actions)
}

pub(super) fn project_raw_stream_llm_response_actions(
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<Vec<SemanticAction>> {
    let body = parse_llm_response_body(bytes)?;
    let first = *segments.first()?;
    let attributes = raw_llm_response_attributes(segments, bytes, &body);
    let response = SemanticAction {
        action_id: llm_raw_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status: llm_response_status(segments, false, &body),
        completeness: llm_response_completeness(segments, false, &body),
        confidence_millis: None,
        attributes,
        evidence: payload_evidence(segments),
    };
    let sse_actions = sse_actions_for_response(&response, &body, segments);
    let mut actions = vec![response];
    actions.extend(sse_actions);
    Some(actions)
}

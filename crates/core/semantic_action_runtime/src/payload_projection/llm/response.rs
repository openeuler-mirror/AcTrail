//! LLM response projection from inbound plaintext payloads.

use config_core::daemon::SemanticRetentionConfig;
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

const HTTP1_LINE_ENDING: &[u8] = b"\r\n";

pub(super) struct RawChunkedResponseProjection {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) encoded_len: usize,
    pub(super) trailing_chunk_framing: bool,
}

pub(super) fn project_stream_llm_response_message_actions(
    config: &SemanticRetentionConfig,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpResponseParts,
    segments: &[&PayloadSegment],
) -> Option<Vec<SemanticAction>> {
    let body = parse_llm_response_body(&http.body)?;
    let first = *segments.first()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let attributes = llm_response_attributes(config, segments, raw_bytes, &http, &body);
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
    let sse_actions = sse_actions_for_response(config, &response, &body, segments);
    let mut actions = vec![response];
    actions.extend(sse_actions);
    Some(actions)
}

pub(super) fn project_raw_chunked_stream_llm_response_actions(
    config: &SemanticRetentionConfig,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<RawChunkedResponseProjection> {
    let chunked = parse_chunked_body_prefix(bytes)?;
    let body = parse_llm_response_body(&chunked.body)?;
    let first = *segments.first()?;
    let attributes = raw_llm_response_attributes(config, segments, &chunked.body, &body);
    let response = SemanticAction {
        action_id: llm_raw_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status: llm_response_status(segments, chunked.complete, &body),
        completeness: llm_response_completeness(segments, chunked.complete, &body),
        confidence_millis: None,
        attributes,
        evidence: payload_evidence(segments),
    };
    let sse_actions = sse_actions_for_response(config, &response, &body, segments);
    let mut actions = vec![response];
    actions.extend(sse_actions);
    Some(RawChunkedResponseProjection {
        actions,
        encoded_len: chunked.encoded_len,
        trailing_chunk_framing: !chunked.complete,
    })
}

pub(super) fn project_raw_stream_llm_response_actions(
    config: &SemanticRetentionConfig,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<Vec<SemanticAction>> {
    let body = parse_llm_response_body(bytes)?;
    let first = *segments.first()?;
    let attributes = raw_llm_response_attributes(config, segments, bytes, &body);
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
    let sse_actions = sse_actions_for_response(config, &response, &body, segments);
    let mut actions = vec![response];
    actions.extend(sse_actions);
    Some(actions)
}

struct ChunkedBodyPrefix {
    body: Vec<u8>,
    encoded_len: usize,
    complete: bool,
}

fn parse_chunked_body_prefix(bytes: &[u8]) -> Option<ChunkedBodyPrefix> {
    let mut cursor = 0;
    let mut body = Vec::new();
    loop {
        let line_len = find_bytes(&bytes[cursor..], HTTP1_LINE_ENDING)?;
        let line_end = cursor.checked_add(line_len)?;
        let line = std::str::from_utf8(&bytes[cursor..line_end]).ok()?;
        let size_text = line.split_once(';').map(|(size, _)| size).unwrap_or(line);
        let chunk_size = usize::from_str_radix(size_text.trim(), 16).ok()?;
        cursor = line_end.checked_add(HTTP1_LINE_ENDING.len())?;
        if chunk_size == 0 {
            let terminator_end = cursor.checked_add(HTTP1_LINE_ENDING.len())?;
            if bytes.len() >= terminator_end {
                return Some(ChunkedBodyPrefix {
                    body,
                    encoded_len: terminator_end,
                    complete: true,
                });
            }
            return (!body.is_empty()).then_some(ChunkedBodyPrefix {
                body,
                encoded_len: cursor,
                complete: false,
            });
        }
        let chunk_end = cursor.checked_add(chunk_size)?;
        let available_end = bytes.len().min(chunk_end);
        body.extend_from_slice(bytes.get(cursor..available_end)?);
        if available_end < chunk_end {
            return (!body.is_empty()).then_some(ChunkedBodyPrefix {
                body,
                encoded_len: available_end,
                complete: false,
            });
        }
        let after_chunk = chunk_end.checked_add(HTTP1_LINE_ENDING.len())?;
        if bytes.len() < after_chunk {
            return Some(ChunkedBodyPrefix {
                body,
                encoded_len: chunk_end,
                complete: false,
            });
        }
        if bytes.get(chunk_end..after_chunk)? != HTTP1_LINE_ENDING {
            return None;
        }
        cursor = after_chunk;
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

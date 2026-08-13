//! LLM response projection from inbound plaintext payloads.

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus};

use crate::payload_projection::http::HttpResponseParts;

use super::body::{
    IncrementalSseCache, SseBodySource, parse_llm_response_body_incremental,
    parse_llm_response_progress,
};
use super::codec::LlmCodecRegistry;
use super::live_projection::semantic_payload_draft;
use super::response_support::{
    http_response_can_evict, llm_raw_stream_action_id, llm_response_attributes,
    llm_response_completeness, llm_response_status_from_progress, llm_response_title,
    llm_stream_action_id, payload_evidence, plaintext_transport_scheme,
    raw_llm_response_attributes,
};
use super::stream::PayloadStreamGroupKey;

const HTTP1_LINE_ENDING: &[u8] = b"\r\n";

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
    pub(super) payload_segments: Vec<PayloadSegment>,
    pub(super) in_flight: Option<InFlightResponse>,
    pub(super) encoded_len: usize,
    pub(super) terminal: bool,
    pub(super) raw_response: bool,
}

fn in_flight_projection(
    encoded_len: usize,
    message_start: usize,
    raw_response: bool,
) -> LlmResponseProjection {
    LlmResponseProjection {
        actions: Vec::new(),
        payload_segments: Vec::new(),
        in_flight: Some(InFlightResponse { message_start }),
        encoded_len,
        terminal: false,
        raw_response,
    }
}

pub(super) fn project_stream_llm_response_message_actions(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    mut http: HttpResponseParts,
    segments: &[&PayloadSegment],
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LlmResponseProjection> {
    let progress =
        parse_llm_response_progress(SseBodySource::SplitHttp, &http.body, codecs, sse_cache)?;
    let first = *segments.first()?;
    http.scheme = plaintext_transport_scheme(first.source_boundary);
    let status = llm_response_status_from_progress(segments, http.complete, &progress);
    if !force_terminal && status == SemanticActionStatus::InProgress {
        return Some(in_flight_projection(http.encoded_len, message_start, false));
    }
    let body = parse_llm_response_body_incremental(
        SseBodySource::SplitHttp,
        &http.body,
        codecs,
        sse_cache,
    )?;
    let attributes = llm_response_attributes(config, segments, raw_bytes, &http, &body);
    let evidence = payload_evidence(segments);
    let payload_segments = semantic_response_payloads(config, first, raw_bytes);
    let response = SemanticAction {
        action_id: llm_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status,
        completeness: llm_response_completeness(segments, http.complete, &body),
        confidence_millis: None,
        attributes,
        evidence,
    };
    Some(LlmResponseProjection {
        actions: vec![response],
        payload_segments,
        in_flight: None,
        encoded_len: http.encoded_len,
        terminal: http_response_can_evict(&http) && status != SemanticActionStatus::InProgress,
        raw_response: false,
    })
}

pub(super) fn project_raw_chunked_stream_llm_response_actions(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LlmResponseProjection> {
    let chunked = parse_chunked_body_prefix(bytes)?;
    let progress =
        parse_llm_response_progress(SseBodySource::ChunkedBody, &chunked.body, codecs, sse_cache)?;
    let first = *segments.first()?;
    let status = llm_response_status_from_progress(segments, chunked.complete, &progress);
    if !force_terminal && status == SemanticActionStatus::InProgress {
        return Some(in_flight_projection(
            chunked.encoded_len,
            message_start,
            !chunked.complete,
        ));
    }
    let body = parse_llm_response_body_incremental(
        SseBodySource::ChunkedBody,
        &chunked.body,
        codecs,
        sse_cache,
    )?;
    let attributes = raw_llm_response_attributes(config, segments, &chunked.body, &body);
    let payload_segments = semantic_response_payloads(config, first, &chunked.body);
    let response = SemanticAction {
        action_id: llm_raw_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status,
        completeness: llm_response_completeness(segments, chunked.complete, &body),
        confidence_millis: None,
        attributes,
        evidence: payload_evidence(segments),
    };
    Some(LlmResponseProjection {
        actions: vec![response],
        payload_segments,
        in_flight: None,
        encoded_len: chunked.encoded_len,
        terminal: status != SemanticActionStatus::InProgress,
        raw_response: !chunked.complete,
    })
}

pub(super) fn project_raw_stream_llm_response_actions(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LlmResponseProjection> {
    let progress = parse_llm_response_progress(SseBodySource::RawBytes, bytes, codecs, sse_cache)?;
    let first = *segments.first()?;
    let status = llm_response_status_from_progress(segments, false, &progress);
    if !force_terminal && status == SemanticActionStatus::InProgress {
        return Some(in_flight_projection(bytes.len(), message_start, true));
    }
    let body =
        parse_llm_response_body_incremental(SseBodySource::RawBytes, bytes, codecs, sse_cache)?;
    let attributes = raw_llm_response_attributes(config, segments, bytes, &body);
    let payload_segments = semantic_response_payloads(config, first, bytes);
    let response = SemanticAction {
        action_id: llm_raw_stream_action_id(key, message_start, first),
        trace_id: first.trace_id,
        kind: SemanticActionKind::LlmResponse,
        title: llm_response_title(&attributes),
        start_time: first.observed_at,
        end_time: segments.last().map(|segment| segment.observed_at),
        process: first.process.clone(),
        status,
        completeness: llm_response_completeness(segments, false, &body),
        confidence_millis: None,
        attributes,
        evidence: payload_evidence(segments),
    };
    Some(LlmResponseProjection {
        actions: vec![response],
        payload_segments,
        in_flight: None,
        encoded_len: bytes.len(),
        terminal: true,
        raw_response: true,
    })
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

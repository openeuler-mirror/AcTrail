//! Single-message LLM projection used by live incremental stream state.

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::{PayloadRedactionState, PayloadSegment};
use semantic_action::{LlmRequestContentWrite, SemanticAction};

use crate::payload_projection::http::{
    HttpRequestParts, HttpResponseParts, request_prefix_skip_len, request_stream_id_hint,
    split_request, split_response,
};

use super::body::IncrementalSseCache;
use super::codec::LlmCodecRegistry;
use super::request::ProjectedLlmRequestHistory;
use super::request::project_stream_llm_request_action;
use super::response::{
    InFlightResponse, LlmResponseProjection, ProjectedProviderResponseId,
    project_raw_chunked_stream_llm_response_actions, project_raw_stream_llm_response_actions,
    project_stream_llm_response_message_actions,
};
use super::response_support::http_response_can_evict;
use super::stream::PayloadStreamGroupKey;

pub(crate) struct LiveLlmProjection {
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) llm_request_contents: Vec<LlmRequestContentWrite>,
    pub(crate) llm_request_histories: Vec<ProjectedLlmRequestHistory>,
    pub(crate) provider_response_ids: Vec<ProjectedProviderResponseId>,
    pub(crate) payload_segments: Vec<PayloadSegment>,
    pub(crate) in_flight: Option<InFlightResponse>,
    pub(crate) encoded_len: usize,
    pub(crate) terminal: bool,
    pub(crate) raw_response: bool,
}

pub(crate) struct LiveLlmResponseMessage {
    http: Option<crate::payload_projection::http::HttpResponseParts>,
    encoded_len: usize,
}

impl LiveLlmResponseMessage {
    pub(crate) fn parse(bytes: &[u8]) -> Self {
        let http = split_response(bytes);
        let encoded_len = http
            .as_ref()
            .map_or_else(|| bytes.len(), |http| http.encoded_len);
        Self { http, encoded_len }
    }

    pub(crate) fn encoded_len(&self) -> usize {
        self.encoded_len
    }
}

pub(crate) fn live_llm_request_message_len(bytes: &[u8]) -> Option<usize> {
    split_request(bytes).map(|http| http.encoded_len)
}

pub(crate) fn live_llm_request_stream_id_hint(bytes: &[u8]) -> Option<Option<u32>> {
    request_stream_id_hint(bytes)
}

pub(crate) fn live_llm_request_prefix_skip_len(bytes: &[u8]) -> Option<usize> {
    request_prefix_skip_len(bytes)
}

pub(crate) fn project_live_llm_request_message(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<LiveLlmProjection> {
    let http = split_request(bytes)?;
    let encoded_len = http.encoded_len;
    let raw_bytes = bytes.get(..encoded_len)?;
    let request = project_stream_llm_request_action(
        config,
        codecs,
        key,
        message_start,
        raw_bytes,
        http,
        segments,
    );
    let (actions, llm_request_contents, llm_request_histories, payload_segments) = match request {
        Some(request) => (
            vec![request.action],
            request.content.into_iter().collect::<Vec<_>>(),
            request.trajectory_history.into_iter().collect::<Vec<_>>(),
            request.payload_segments,
        ),
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };
    Some(LiveLlmProjection {
        actions,
        llm_request_contents,
        llm_request_histories,
        provider_response_ids: Vec::new(),
        payload_segments,
        in_flight: None,
        encoded_len,
        terminal: true,
        raw_response: false,
    })
}

pub(crate) fn project_live_llm_response_message(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    message: LiveLlmResponseMessage,
    segments: &[&PayloadSegment],
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LiveLlmProjection> {
    if let Some(http) = message.http {
        let encoded_len = http.encoded_len;
        let raw_bytes = bytes.get(..encoded_len)?;
        let can_evict = http_response_can_evict(&http);
        let Some(projection) = project_stream_llm_response_message_actions(
            config,
            codecs,
            key,
            message_start,
            raw_bytes,
            http,
            segments,
            sse_cache,
            force_terminal,
        ) else {
            return can_evict.then_some(empty_terminal_projection(encoded_len));
        };
        return Some(live_projection_from_response(projection));
    }

    if let Some(projection) = project_raw_chunked_stream_llm_response_actions(
        config,
        codecs,
        key,
        message_start,
        bytes,
        segments,
        sse_cache,
        force_terminal,
    ) {
        return Some(live_projection_from_response(projection));
    }

    let projection = project_raw_stream_llm_response_actions(
        config,
        codecs,
        key,
        message_start,
        bytes,
        segments,
        sse_cache,
        force_terminal,
    )?;
    Some(live_projection_from_response(projection))
}

/// Project one de-multiplexed HTTP/2 stream's request body (the stream's DATA
/// payloads) as an `llm.request`, tagged with the HTTP/2 `stream_id` so the
/// request pairs with the response on the same stream.
pub(crate) fn project_http2_stream_request(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    stream_id: u32,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<LiveLlmProjection> {
    let encoded_len = bytes.len();
    let http = HttpRequestParts {
        protocol: "h2",
        scheme: "https",
        method: None,
        authority: None,
        path: None,
        stream_id: Some(stream_id),
        headers_text: None,
        headers_hpack_base64: None,
        body: bytes.to_vec(),
        encoded_len,
    };
    let request = project_stream_llm_request_action(
        config,
        codecs,
        key,
        message_start,
        bytes,
        http,
        segments,
    );
    let (actions, llm_request_contents, llm_request_histories, payload_segments) = match request {
        Some(request) => (
            vec![request.action],
            request.content.into_iter().collect::<Vec<_>>(),
            request.trajectory_history.into_iter().collect::<Vec<_>>(),
            request.payload_segments,
        ),
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };
    Some(LiveLlmProjection {
        actions,
        llm_request_contents,
        llm_request_histories,
        provider_response_ids: Vec::new(),
        payload_segments,
        in_flight: None,
        encoded_len,
        terminal: true,
        raw_response: false,
    })
}

/// Project one de-multiplexed HTTP/2 stream's response body (the stream's DATA
/// payloads) as an `llm.response`, tagged with the HTTP/2 `stream_id`. The
/// response is terminal when `end_stream` was seen or the SSE body reached a
/// completion marker; until then it stays `InProgress` and is not evicted.
pub(crate) fn project_http2_stream_response(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    stream_id: u32,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
    sse_cache: &mut Option<IncrementalSseCache>,
    end_stream: bool,
) -> Option<LiveLlmProjection> {
    let encoded_len = bytes.len();
    let http = HttpResponseParts {
        protocol: "h2",
        scheme: "https",
        status_code: None,
        reason: None,
        stream_id: Some(stream_id),
        headers_text: None,
        headers_hpack_base64: None,
        body: bytes.to_vec(),
        encoded_len,
        complete: end_stream,
        body_boundary_known: false,
    };
    let projection = project_stream_llm_response_message_actions(
        config,
        codecs,
        key,
        message_start,
        bytes,
        http,
        segments,
        sse_cache,
        end_stream,
    )?;
    Some(live_projection_from_response(projection))
}

fn live_projection_from_response(projection: LlmResponseProjection) -> LiveLlmProjection {
    LiveLlmProjection {
        actions: projection.actions,
        llm_request_contents: Vec::new(),
        llm_request_histories: Vec::new(),
        provider_response_ids: projection.provider_response_ids,
        payload_segments: projection.payload_segments,
        in_flight: projection.in_flight,
        encoded_len: projection.encoded_len,
        terminal: projection.terminal,
        raw_response: projection.raw_response,
    }
}

fn empty_terminal_projection(encoded_len: usize) -> LiveLlmProjection {
    LiveLlmProjection {
        actions: Vec::new(),
        llm_request_contents: Vec::new(),
        llm_request_histories: Vec::new(),
        provider_response_ids: Vec::new(),
        payload_segments: Vec::new(),
        in_flight: None,
        encoded_len,
        terminal: true,
        raw_response: false,
    }
}

/// Build a semantic-exchange payload record from the assembled message bytes.
///
/// Emitted only when transport segments are not persisted (L4 payload retention
/// disabled): the exchange's assembled request/response bytes are written as a
/// single payload row reusing the first segment id, which the action payload
/// evidence already references.
pub(crate) fn semantic_payload_draft(
    first: &PayloadSegment,
    assembled_bytes: &[u8],
) -> PayloadSegment {
    let size = u64::try_from(assembled_bytes.len()).unwrap_or(u64::MAX);
    PayloadSegment {
        segment_id: first.segment_id,
        trace_id: first.trace_id,
        observed_at: first.observed_at,
        process: first.process.clone(),
        source_boundary: first.source_boundary,
        content_state: first.content_state,
        direction: first.direction,
        stream_key: first.stream_key.clone(),
        sequence: first.sequence,
        original_size: size,
        captured_size: size,
        operation_id: first.operation_id,
        operation_offset: 0,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: first.operation_completion_state,
        truncation: first.truncation,
        redaction: PayloadRedactionState::NotRequired,
        library: first.library.clone(),
        symbol: first.symbol.clone(),
        protocol_hint: first.protocol_hint.clone(),
        bytes: assembled_bytes.to_vec(),
    }
}

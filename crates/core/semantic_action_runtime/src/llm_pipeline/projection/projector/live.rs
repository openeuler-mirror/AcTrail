//! Single-message LLM projection used by live incremental stream state.

use std::sync::Arc;

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{LlmRequestContentWrite, SemanticAction};

use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::response::IncrementalSseCache;
use crate::llm_pipeline::transport::evidence::EvidenceSnapshot;
use crate::llm_pipeline::transport::http1::DecodedHttp1Message;
use crate::llm_pipeline::transport::{HttpRequestParts, HttpResponseParts};

use super::request::project_stream_llm_request_action;
use super::request::{ProjectedLlmRequestHistory, ProjectedLlmToolResult};
use super::response::{
    InFlightResponse, LlmResponseProjection, ProjectedProviderResponseId,
    project_raw_stream_llm_response_actions, project_stream_llm_response_message_actions,
};

pub(crate) struct LiveLlmProjection {
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) llm_request_contents: Vec<LlmRequestContentWrite>,
    pub(crate) llm_request_histories: Vec<ProjectedLlmRequestHistory>,
    pub(crate) llm_tool_results: Vec<ProjectedLlmToolResult>,
    pub(crate) provider_response_ids: Vec<ProjectedProviderResponseId>,
    pub(crate) payload_segments: Vec<PayloadSegment>,
    pub(crate) in_flight: Option<InFlightResponse>,
    pub(crate) encoded_len: usize,
    pub(crate) terminal: bool,
}

pub(in crate::llm_pipeline) fn project_decoded_http1_request(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    message: DecodedHttp1Message,
    segments: &[&PayloadSegment],
) -> Option<LiveLlmProjection> {
    let encoded_len = message.encoded_len;
    let http = HttpRequestParts {
        protocol: message.protocol,
        scheme: "https",
        method: message.method,
        authority: message.authority,
        path: message.path,
        stream_id: None,
        headers_text: Some(message.headers_text),
        headers_hpack_base64: None,
        body: message.body,
        encoded_len,
    };
    let request = project_stream_llm_request_action(
        config,
        codecs,
        key,
        message_start,
        raw_bytes,
        http,
        segments,
    );
    let (actions, llm_request_contents, llm_request_histories, llm_tool_results, payload_segments) =
        match request {
            Some(request) => (
                vec![request.action],
                request.content.into_iter().collect::<Vec<_>>(),
                request.trajectory_history.into_iter().collect::<Vec<_>>(),
                request.tool_results,
                request.payload_segments,
            ),
            None => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        };
    Some(LiveLlmProjection {
        actions,
        llm_request_contents,
        llm_request_histories,
        llm_tool_results,
        provider_response_ids: Vec::new(),
        payload_segments,
        in_flight: None,
        encoded_len,
        terminal: true,
    })
}

pub(in crate::llm_pipeline) fn project_raw_llm_response_message(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    evidence: &EvidenceSnapshot,
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LiveLlmProjection> {
    project_raw_stream_llm_response_actions(
        config,
        codecs,
        key,
        message_start,
        bytes,
        evidence,
        sse_cache,
        force_terminal,
    )
    .map(live_projection_from_response)
}

pub(in crate::llm_pipeline) fn project_decoded_http1_response(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    raw_bytes: &[u8],
    message: DecodedHttp1Message,
    evidence: &EvidenceSnapshot,
    sse_cache: &mut Option<IncrementalSseCache>,
    force_terminal: bool,
) -> Option<LiveLlmProjection> {
    let encoded_len = message.encoded_len;
    let http = HttpResponseParts {
        protocol: message.protocol,
        scheme: "https",
        status_code: message.status_code,
        reason: message.reason,
        stream_id: None,
        headers_text: Some(message.headers_text),
        headers_hpack_base64: None,
        body: message.body,
        encoded_len,
        complete: message.complete,
        body_boundary_known: message.body_boundary_known,
    };
    let projection = project_stream_llm_response_message_actions(
        config,
        codecs,
        key,
        message_start,
        raw_bytes,
        http,
        evidence,
        sse_cache,
        force_terminal,
    )?;
    Some(live_projection_from_response(projection))
}

/// Project one de-multiplexed HTTP/2 stream's request body (the stream's DATA
/// payloads) as an `llm.request`, tagged with the HTTP/2 `stream_id` so the
pub(crate) fn project_http2_stream_request(
    config: &SemanticRetentionConfig,
    codecs: &LlmCodecRegistry,
    key: &PayloadStreamGroupKey,
    stream_id: u32,
    message_start: usize,
    bytes: &[u8],
    body: Arc<Vec<u8>>,
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
        body,
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
    let (actions, llm_request_contents, llm_request_histories, llm_tool_results, payload_segments) =
        match request {
            Some(request) => (
                vec![request.action],
                request.content.into_iter().collect::<Vec<_>>(),
                request.trajectory_history.into_iter().collect::<Vec<_>>(),
                request.tool_results,
                request.payload_segments,
            ),
            None => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        };
    Some(LiveLlmProjection {
        actions,
        llm_request_contents,
        llm_request_histories,
        llm_tool_results,
        provider_response_ids: Vec::new(),
        payload_segments,
        in_flight: None,
        encoded_len,
        terminal: true,
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
    body: Arc<Vec<u8>>,
    evidence: &EvidenceSnapshot,
    sse_cache: &mut Option<IncrementalSseCache>,
    transport_complete: bool,
    force_materialize: bool,
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
        body,
        encoded_len,
        complete: transport_complete,
        body_boundary_known: false,
    };
    let projection = project_stream_llm_response_message_actions(
        config,
        codecs,
        key,
        message_start,
        bytes,
        http,
        evidence,
        sse_cache,
        force_materialize,
    )?;
    Some(live_projection_from_response(projection))
}

fn live_projection_from_response(projection: LlmResponseProjection) -> LiveLlmProjection {
    LiveLlmProjection {
        actions: projection.actions,
        llm_request_contents: Vec::new(),
        llm_request_histories: Vec::new(),
        llm_tool_results: Vec::new(),
        provider_response_ids: projection.provider_response_ids,
        payload_segments: projection.payload_segments,
        in_flight: projection.in_flight,
        encoded_len: projection.encoded_len,
        terminal: projection.terminal,
    }
}

pub(in crate::llm_pipeline) fn empty_terminal_projection(encoded_len: usize) -> LiveLlmProjection {
    LiveLlmProjection {
        actions: Vec::new(),
        llm_request_contents: Vec::new(),
        llm_request_histories: Vec::new(),
        llm_tool_results: Vec::new(),
        provider_response_ids: Vec::new(),
        payload_segments: Vec::new(),
        in_flight: None,
        encoded_len,
        terminal: true,
    }
}

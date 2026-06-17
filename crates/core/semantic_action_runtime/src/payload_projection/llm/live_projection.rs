//! Single-message LLM projection used by live incremental stream state.

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus};

use crate::payload_projection::http::{request_stream_id_hint, split_request, split_response};

use super::request::project_stream_llm_request_action;
use super::response::{
    project_raw_chunked_stream_llm_response_actions, project_raw_stream_llm_response_actions,
    project_stream_llm_response_message_actions,
};
use super::stream::PayloadStreamGroupKey;

pub(crate) struct LiveLlmProjection {
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) encoded_len: usize,
    pub(crate) terminal: bool,
    pub(crate) raw_response: bool,
}

pub(crate) fn live_llm_request_message_len(bytes: &[u8]) -> Option<usize> {
    split_request(bytes).map(|http| http.encoded_len)
}

pub(crate) fn live_llm_request_stream_id_hint(bytes: &[u8]) -> Option<Option<u32>> {
    request_stream_id_hint(bytes)
}

pub(crate) fn live_llm_http_response_message_len(bytes: &[u8]) -> Option<usize> {
    split_response(bytes).map(|http| http.encoded_len)
}

pub(crate) fn project_live_llm_request_message(
    config: &SemanticRetentionConfig,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<LiveLlmProjection> {
    let http = split_request(bytes)?;
    let encoded_len = http.encoded_len;
    let raw_bytes = bytes.get(..encoded_len)?;
    let action =
        project_stream_llm_request_action(config, key, message_start, raw_bytes, http, segments);
    Some(LiveLlmProjection {
        actions: action.into_iter().collect(),
        encoded_len,
        terminal: true,
        raw_response: false,
    })
}

pub(crate) fn project_live_llm_response_message(
    config: &SemanticRetentionConfig,
    key: &PayloadStreamGroupKey,
    message_start: usize,
    bytes: &[u8],
    segments: &[&PayloadSegment],
) -> Option<LiveLlmProjection> {
    if let Some(http) = split_response(bytes) {
        let encoded_len = http.encoded_len;
        let raw_bytes = bytes.get(..encoded_len)?;
        let can_evict = http_response_can_evict(&http);
        let Some(actions) = project_stream_llm_response_message_actions(
            config,
            key,
            message_start,
            raw_bytes,
            http,
            segments,
        ) else {
            return can_evict.then_some(LiveLlmProjection {
                actions: Vec::new(),
                encoded_len,
                terminal: true,
                raw_response: false,
            });
        };
        return Some(response_projection(actions, encoded_len, can_evict, false));
    }

    if let Some(projection) =
        project_raw_chunked_stream_llm_response_actions(config, key, message_start, bytes, segments)
    {
        return Some(response_projection(
            projection.actions,
            projection.encoded_len,
            true,
            projection.trailing_chunk_framing,
        ));
    }

    let actions =
        project_raw_stream_llm_response_actions(config, key, message_start, bytes, segments)?;
    Some(response_projection(actions, bytes.len(), true, false))
}

fn response_projection(
    actions: Vec<SemanticAction>,
    encoded_len: usize,
    can_evict: bool,
    raw_response: bool,
) -> LiveLlmProjection {
    let terminal = can_evict
        && actions
            .iter()
            .find(|action| action.kind == SemanticActionKind::LlmResponse)
            .is_some_and(|action| action.status != SemanticActionStatus::InProgress);
    LiveLlmProjection {
        actions,
        encoded_len,
        terminal,
        raw_response,
    }
}

fn http_response_can_evict(http: &crate::payload_projection::http::HttpResponseParts) -> bool {
    !http.body_boundary_known || http.complete
}

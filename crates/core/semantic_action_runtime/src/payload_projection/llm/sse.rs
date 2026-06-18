//! SSE protocol detail actions derived from LLM response bodies.

use std::collections::BTreeMap;

use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind, attr_keys as attrs};

use super::body::{LlmResponseBody, sse_events_json};

pub(super) const ATTR_LLM_RESPONSE_ACTION_ID: &str = attrs::llm_response::ACTION_ID;

pub(super) fn sse_actions_for_response(
    config: &SemanticRetentionConfig,
    response: &SemanticAction,
    body: &LlmResponseBody,
    segments: &[&PayloadSegment],
) -> Vec<SemanticAction> {
    if body.sse_events.is_empty() {
        return Vec::new();
    }
    let event_content = config.sse_event_content_for_llm_response();
    if !config.sse_stream_summary_enabled()
        && sse_events_json(&body.sse_events, event_content).is_none()
    {
        return Vec::new();
    }
    let Some(first) = segments.first() else {
        return Vec::new();
    };
    let stream_action_id = sse_stream_action_id(&response.action_id);
    vec![sse_stream_action(
        config,
        response,
        body,
        first,
        segments.last().copied(),
        &stream_action_id,
    )]
}

fn sse_stream_action(
    config: &SemanticRetentionConfig,
    response: &SemanticAction,
    body: &LlmResponseBody,
    first: &PayloadSegment,
    last: Option<&PayloadSegment>,
    action_id: &str,
) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        (
            ATTR_LLM_RESPONSE_ACTION_ID.to_string(),
            response.action_id.clone(),
        ),
        (
            attrs::payload::STREAM_KEY.to_string(),
            first.stream_key.to_string(),
        ),
    ]);
    if config.sse_stream_summary_enabled() {
        attributes.insert(
            attrs::sse::EVENT_COUNT.to_string(),
            body.sse_events.len().to_string(),
        );
        attributes.insert(
            attrs::sse::CONTENT_DELTA_COUNT.to_string(),
            sse_content_delta_count(body).to_string(),
        );
        attributes.insert(
            attrs::sse::REASONING_DELTA_COUNT.to_string(),
            sse_reasoning_delta_count(body).to_string(),
        );
        attributes.insert(
            attrs::sse::TOOL_DELTA_COUNT.to_string(),
            sse_tool_delta_count(body).to_string(),
        );
        attributes.insert(attrs::sse::DONE.to_string(), body.done.to_string());
    }
    if let Some(events_json) = sse_events_json(
        &body.sse_events,
        config.sse_event_content_for_llm_response(),
    ) {
        attributes.insert(attrs::sse::EVENTS_JSON.to_string(), events_json);
    }
    if let Some(model) = body.model.as_deref() {
        attributes.insert(attrs::llm_response::MODEL.to_string(), model.to_string());
    }
    SemanticAction {
        action_id: action_id.to_string(),
        trace_id: response.trace_id,
        kind: SemanticActionKind::SseStream,
        title: "SSE stream".to_string(),
        start_time: first.observed_at,
        end_time: last.map(|segment| segment.observed_at),
        process: response.process.clone(),
        status: response.status,
        completeness: response.completeness,
        confidence_millis: None,
        attributes,
        evidence: response.evidence.clone(),
    }
}

fn sse_stream_action_id(response_action_id: &str) -> String {
    format!("{response_action_id}:sse.stream")
}

fn sse_content_delta_count(body: &LlmResponseBody) -> usize {
    body.sse_events
        .iter()
        .filter(|event| event.has_content_delta)
        .count()
}

fn sse_reasoning_delta_count(body: &LlmResponseBody) -> usize {
    body.sse_events
        .iter()
        .filter(|event| event.has_reasoning_delta)
        .count()
}

fn sse_tool_delta_count(body: &LlmResponseBody) -> usize {
    body.sse_events
        .iter()
        .filter(|event| event.has_tool_delta)
        .count()
}

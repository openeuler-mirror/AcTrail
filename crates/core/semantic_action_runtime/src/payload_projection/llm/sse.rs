//! SSE protocol detail actions derived from LLM response bodies.

use std::collections::BTreeMap;

use model_core::payload::PayloadSegment;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind,
};

use super::body::{LlmResponseBody, SseEvent};

pub(super) const ATTR_LLM_RESPONSE_ACTION_ID: &str = "llm.response.action_id";
pub(super) const ATTR_SSE_STREAM_ACTION_ID: &str = "sse.stream.action_id";

pub(super) fn sse_actions_for_response(
    response: &SemanticAction,
    body: &LlmResponseBody,
    segments: &[&PayloadSegment],
) -> Vec<SemanticAction> {
    if body.sse_events.is_empty() {
        return Vec::new();
    }
    let Some(first) = segments.first() else {
        return Vec::new();
    };
    let stream_action_id = sse_stream_action_id(&response.action_id);
    let mut actions = vec![sse_stream_action(
        response,
        body,
        first,
        segments.last().copied(),
        &stream_action_id,
    )];
    actions.extend(body.sse_events.iter().map(|event| {
        sse_event_action(
            response,
            event,
            segments,
            &stream_action_id,
            &sse_event_action_id(&stream_action_id, event.index),
        )
    }));
    actions
}

fn sse_stream_action(
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
            "sse.event_count".to_string(),
            body.sse_events.len().to_string(),
        ),
        ("sse.done".to_string(), body.done.to_string()),
        (
            "payload.stream_key".to_string(),
            first.stream_key.to_string(),
        ),
    ]);
    if let Some(model) = body.model.as_deref() {
        attributes.insert("llm.response.model".to_string(), model.to_string());
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

fn sse_event_action(
    response: &SemanticAction,
    event: &SseEvent,
    segments: &[&PayloadSegment],
    stream_action_id: &str,
    action_id: &str,
) -> SemanticAction {
    let evidence = sse_event_evidence(event, segments);
    let event_time = evidence
        .first()
        .and_then(|evidence| segment_time(evidence.id, segments))
        .unwrap_or(response.start_time);
    let mut attributes = BTreeMap::from([
        (
            ATTR_SSE_STREAM_ACTION_ID.to_string(),
            stream_action_id.to_string(),
        ),
        ("sse.event.index".to_string(), event.index.to_string()),
        ("sse.data_text".to_string(), event.data.clone()),
        ("sse.done".to_string(), event.done.to_string()),
    ]);
    if let Some(event_type) = event.event_type.as_deref() {
        attributes.insert("sse.event.type".to_string(), event_type.to_string());
    }
    if let Some(id) = event.id.as_deref() {
        attributes.insert("sse.event.id".to_string(), id.to_string());
    }
    if let Some(value) = &event.json {
        attributes.insert("sse.data_json".to_string(), value.to_string());
        attributes.insert("sse.data_json_state".to_string(), "valid".to_string());
    } else {
        attributes.insert(
            "sse.data_json_state".to_string(),
            "invalid_or_unavailable".to_string(),
        );
    }
    if let Some(model) = event.model.as_deref() {
        attributes.insert("llm.response.model".to_string(), model.to_string());
    }
    if let Some(text) = event.content_text.as_deref() {
        attributes.insert(
            "llm.response.delta.content_text".to_string(),
            text.to_string(),
        );
    }
    if let Some(text) = event.reasoning_text.as_deref() {
        attributes.insert(
            "llm.response.delta.reasoning_text".to_string(),
            text.to_string(),
        );
    }
    if let Some(tool_calls_json) = event.tool_calls_json.as_deref() {
        attributes.insert(
            "llm.response.delta.tool_calls_json".to_string(),
            tool_calls_json.to_string(),
        );
    }
    if let Some(finish_reason) = event.finish_reason.as_deref() {
        attributes.insert(
            "llm.response.finish_reason".to_string(),
            finish_reason.to_string(),
        );
    }
    SemanticAction {
        action_id: action_id.to_string(),
        trace_id: response.trace_id,
        kind: SemanticActionKind::SseEvent,
        title: sse_event_title(event),
        start_time: event_time,
        end_time: Some(event_time),
        process: response.process.clone(),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence,
    }
}

fn sse_event_title(event: &SseEvent) -> String {
    if event.done {
        return "SSE done".to_string();
    }
    if event.tool_calls_json.is_some() {
        return "SSE tool delta".to_string();
    }
    if event.content_text.is_some() {
        return "SSE content delta".to_string();
    }
    if event.reasoning_text.is_some() {
        return "SSE reasoning delta".to_string();
    }
    "SSE event".to_string()
}

fn sse_event_evidence(event: &SseEvent, segments: &[&PayloadSegment]) -> Vec<SemanticEvidence> {
    segments
        .iter()
        .find(|segment| segment_contains_event_data(segment, &event.data))
        .copied()
        .or_else(|| segments.first().copied())
        .map(|segment| {
            vec![SemanticEvidence {
                kind: SemanticEvidenceKind::PayloadSegment,
                id: segment.segment_id.get(),
                role: "sse.event.payload".to_string(),
            }]
        })
        .unwrap_or_default()
}

fn segment_contains_event_data(segment: &PayloadSegment, data: &str) -> bool {
    String::from_utf8_lossy(&segment.bytes).contains(data)
}

fn segment_time(segment_id: u64, segments: &[&PayloadSegment]) -> Option<std::time::SystemTime> {
    segments
        .iter()
        .find(|segment| segment.segment_id.get() == segment_id)
        .map(|segment| segment.observed_at)
}

fn sse_stream_action_id(response_action_id: &str) -> String {
    format!("{response_action_id}:sse.stream")
}

fn sse_event_action_id(stream_action_id: &str, event_index: usize) -> String {
    format!("{stream_action_id}:event:{event_index}")
}

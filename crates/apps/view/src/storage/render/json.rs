//! Machine-readable viewer rendering for regression and automation.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::payload::{PayloadDirection, PayloadSegment};
use model_core::process::ProcessIdentity;
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionLink, SemanticEvidence};
use serde_json::{Value, json};

pub(super) fn render_traces(traces: Vec<TraceRecord>) -> Result<String, String> {
    let traces = traces.iter().map(trace_json).collect::<Vec<_>>();
    render(json!({ "traces": traces }))
}

pub(super) fn render_payloads(segments: Vec<PayloadSegment>) -> Result<String, String> {
    let payloads = segments.iter().map(payload_json).collect::<Vec<_>>();
    render(json!({ "payloads": payloads }))
}

pub(super) fn render_semantic_actions(
    actions: Vec<SemanticAction>,
    links: Vec<SemanticActionLink>,
) -> Result<String, String> {
    let action_ids = actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect::<BTreeSet<_>>();
    let actions = actions.iter().map(action_json).collect::<Vec<_>>();
    let links = links
        .iter()
        .filter(|link| {
            action_ids.contains(link.parent_action_id.as_str())
                && action_ids.contains(link.child_action_id.as_str())
        })
        .map(link_json)
        .collect::<Vec<_>>();
    render(json!({
        "actions": actions,
        "links": links,
    }))
}

fn trace_json(trace: &TraceRecord) -> Value {
    json!({
        "trace_id": trace.trace_id.to_string(),
        "trace_id_raw": trace.trace_id.get(),
        "name": trace.display_name.to_string(),
        "profile": trace.profile_name.to_string(),
        "root_process": process_json(&trace.root_process_identity),
        "state": format!("{:?}", trace.lifecycle_state),
        "health": format!("{:?}", trace.health),
        "tags": trace.tags.iter().collect::<Vec<_>>(),
        "created_at_unix_nanos": unix_nanos(trace.timings.created_at),
        "started_at_unix_nanos": trace.timings.started_at.and_then(unix_nanos),
        "completed_at_unix_nanos": trace.timings.completed_at.and_then(unix_nanos),
        "failed_at_unix_nanos": trace.timings.failed_at.and_then(unix_nanos),
    })
}

fn payload_json(segment: &PayloadSegment) -> Value {
    json!({
        "segment_id": segment.segment_id.to_string(),
        "segment_id_raw": segment.segment_id.get(),
        "trace_id": segment.trace_id.to_string(),
        "trace_id_raw": segment.trace_id.get(),
        "observed_at_unix_nanos": unix_nanos(segment.observed_at),
        "process": process_json(&segment.process),
        "source_boundary": format!("{:?}", segment.source_boundary),
        "content_state": format!("{:?}", segment.content_state),
        "direction": payload_direction(segment.direction),
        "stream_key": segment.stream_key.as_str(),
        "sequence": segment.sequence,
        "original_size": segment.original_size,
        "captured_size": segment.captured_size,
        "operation_id": segment.operation_id,
        "operation_offset": segment.operation_offset,
        "operation_original_size": segment.operation_original_size,
        "operation_captured_size": segment.operation_captured_size,
        "operation_completion_state": segment.operation_completion_state.as_str(),
        "truncation": format!("{:?}", segment.truncation),
        "redaction": format!("{:?}", segment.redaction),
        "library": &segment.library,
        "symbol": &segment.symbol,
        "protocol_hint": segment.protocol_hint.as_ref(),
    })
}

fn action_json(action: &SemanticAction) -> Value {
    json!({
        "action_id": &action.action_id,
        "trace_id": action.trace_id.to_string(),
        "trace_id_raw": action.trace_id.get(),
        "kind": action.kind.as_str(),
        "title": &action.title,
        "start_time_unix_nanos": unix_nanos(action.start_time),
        "end_time_unix_nanos": action.end_time.and_then(unix_nanos),
        "process": process_json(&action.process),
        "status": action.status.as_str(),
        "completeness": action.completeness.as_str(),
        "confidence_millis": action.confidence_millis,
        "attributes": &action.attributes,
        "evidence": action.evidence.iter().map(evidence_json).collect::<Vec<_>>(),
    })
}

fn link_json(link: &SemanticActionLink) -> Value {
    json!({
        "trace_id": link.trace_id.to_string(),
        "trace_id_raw": link.trace_id.get(),
        "parent_action_id": &link.parent_action_id,
        "child_action_id": &link.child_action_id,
        "role": link.role.as_str(),
        "confidence": link.confidence.as_str(),
        "attributes": &link.attributes,
        "evidence": link.evidence.iter().map(evidence_json).collect::<Vec<_>>(),
    })
}

fn evidence_json(evidence: &SemanticEvidence) -> Value {
    json!({
        "kind": evidence.kind.as_str(),
        "id": evidence.id,
        "role": &evidence.role,
    })
}

fn process_json(process: &ProcessIdentity) -> Value {
    json!({
        "pid": process.pid,
        "task_id": process.task_id,
        "start_time_ticks": process.start_time_ticks,
        "pid_namespace": process.pid_namespace.as_ref().map(|value| value.as_str()),
        "generation": process.generation,
    })
}

fn payload_direction(direction: PayloadDirection) -> &'static str {
    match direction {
        PayloadDirection::Outbound => "outbound",
        PayloadDirection::Inbound => "inbound",
    }
}

fn unix_nanos(time: SystemTime) -> Option<String> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos().to_string())
}

fn render(value: Value) -> Result<String, String> {
    serde_json::to_string_pretty(&value).map_err(|error| format!("render json failed: {error}"))
}

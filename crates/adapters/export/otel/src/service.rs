//! OTLP JSON document construction.

use std::time::{SystemTime, UNIX_EPOCH};

use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus};
use serde_json::Value;

use crate::serialize::{int_attr, quoted, string_attr};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelExportError {
    pub stage: String,
    pub message: String,
}

impl OtelExportError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

pub fn render_otlp_json(
    trace: &TraceRecord,
    actions: &[SemanticAction],
) -> Result<String, OtelExportError> {
    let compact = render_otlp_json_compact(trace, actions);
    let document = serde_json::from_str::<Value>(&compact)
        .map_err(|error| OtelExportError::new("serialize", error.to_string()))?;
    serde_json::to_string_pretty(&document)
        .map_err(|error| OtelExportError::new("serialize", error.to_string()))
}

pub fn render_otlp_json_line(trace: &TraceRecord, action: &SemanticAction) -> String {
    render_otlp_json_compact(trace, std::slice::from_ref(action))
}

fn render_otlp_json_compact(trace: &TraceRecord, actions: &[SemanticAction]) -> String {
    let service_name = trace.profile_name.as_str();
    let mut spans = Vec::new();
    for action in actions {
        spans.push(render_span(trace, action));
    }
    let resource_attrs = vec![
        string_attr("service.name", service_name),
        string_attr("actrail.trace.display_name", trace.display_name.as_str()),
        string_attr("actrail.trace.profile_name", trace.profile_name.as_str()),
        int_attr("actrail.trace.id", trace.trace_id.get()),
    ];
    format!(
        "{{\"resourceSpans\":[{{\"resource\":{{\"attributes\":[{}]}},\"scopeSpans\":[{{\"scope\":{{\"name\":\"actrail.semantic_actions\",\"version\":\"{}\"}},\"spans\":[{}]}}]}}]}}",
        resource_attrs.join(","),
        env!("CARGO_PKG_VERSION"),
        spans.join(",")
    )
}

fn render_span(trace: &TraceRecord, action: &SemanticAction) -> String {
    let mut attrs = vec![
        string_attr("actrail.action.id", &action.action_id),
        string_attr("actrail.action.kind", action.kind.as_str()),
        string_attr("actrail.action.status", action.status.as_str()),
        string_attr("actrail.action.completeness", action.completeness.as_str()),
        int_attr("process.pid", u64::from(action.process.pid)),
        int_attr("actrail.process.generation", action.process.generation),
    ];
    if let Some(task_id) = action.process.task_id {
        attrs.push(int_attr("process.thread.id", u64::from(task_id)));
    }
    if let Some(namespace) = &action.process.pid_namespace {
        attrs.push(string_attr(
            "actrail.process.pid_namespace",
            namespace.as_str(),
        ));
    }
    if let Some(confidence) = action.confidence_millis {
        attrs.push(int_attr(
            "actrail.action.confidence_millis",
            u64::from(confidence),
        ));
    }
    for (key, value) in &action.attributes {
        attrs.push(string_attr(key, value));
    }

    let events = action
        .evidence
        .iter()
        .map(|evidence| {
            let attrs = vec![
                string_attr("actrail.evidence.kind", evidence.kind.as_str()),
                int_attr("actrail.evidence.id", evidence.id),
                string_attr("actrail.evidence.role", &evidence.role),
            ];
            format!(
                "{{\"name\":\"actrail.evidence\",\"timeUnixNano\":\"{}\",\"attributes\":[{}]}}",
                unix_nanos(action.start_time),
                attrs.join(",")
            )
        })
        .collect::<Vec<_>>();

    format!(
        "{{\"traceId\":{},\"spanId\":{},\"name\":{},\"kind\":\"{}\",\"startTimeUnixNano\":\"{}\",\"endTimeUnixNano\":\"{}\",\"attributes\":[{}],\"events\":[{}],\"status\":{{\"code\":\"{}\"}}}}",
        quoted(&otel_trace_id(trace)),
        quoted(&otel_span_id(&action.action_id)),
        quoted(&action.title),
        span_kind(action.kind),
        unix_nanos(action.start_time),
        unix_nanos(action.end_time.unwrap_or(action.start_time)),
        attrs.join(","),
        events.join(","),
        status_code(action.status)
    )
}

fn span_kind(kind: SemanticActionKind) -> &'static str {
    match kind {
        SemanticActionKind::HttpMessage | SemanticActionKind::LlmRequest => "SPAN_KIND_CLIENT",
        SemanticActionKind::ProcessExec
        | SemanticActionKind::ProcessForkAttempt
        | SemanticActionKind::AgentInvocation
        | SemanticActionKind::FileModify
        | SemanticActionKind::EnforcementDecision => "SPAN_KIND_INTERNAL",
    }
}

fn status_code(status: SemanticActionStatus) -> &'static str {
    match status {
        SemanticActionStatus::Success => "STATUS_CODE_OK",
        SemanticActionStatus::Error => "STATUS_CODE_ERROR",
        SemanticActionStatus::InProgress | SemanticActionStatus::Unknown => "STATUS_CODE_UNSET",
    }
}

fn otel_trace_id(trace: &TraceRecord) -> String {
    format!("{:032x}", trace.trace_id.get())
}

fn otel_span_id(action_id: &str) -> String {
    format!("{:016x}", stable_hash(action_id.as_bytes()))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn unix_nanos(value: SystemTime) -> u128 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

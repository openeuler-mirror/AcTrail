//! OTLP JSON document construction.

use std::time::{SystemTime, UNIX_EPOCH};

use model_core::trace::TraceRecord;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
    SemanticActionStatus,
};
use serde_json::Value;

use crate::serialize::{int_attr, quoted, string_attr};

const ATTR_PROCESS_PARENT_IDENTITY_STATE: &str = "process.parent.identity_state";
const PROCESS_PARENT_IDENTITY_STATE_CONFLICT: &str = "conflict";

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
    links: &[SemanticActionLink],
) -> Result<String, OtelExportError> {
    let compact = render_otlp_json_compact(trace, actions, links);
    let document = serde_json::from_str::<Value>(&compact)
        .map_err(|error| OtelExportError::new("serialize", error.to_string()))?;
    serde_json::to_string_pretty(&document)
        .map_err(|error| OtelExportError::new("serialize", error.to_string()))
}

pub fn render_otlp_json_line(
    trace: &TraceRecord,
    action: &SemanticAction,
    links: &[SemanticActionLink],
) -> String {
    render_otlp_json_compact(trace, std::slice::from_ref(action), links)
}

fn render_otlp_json_compact(
    trace: &TraceRecord,
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
) -> String {
    let service_name = trace.profile_name.as_str();
    let trace_id = otel_trace_id(trace);
    let mut spans = Vec::new();
    for action in actions {
        spans.push(render_span(&trace_id, action, links));
    }
    let mut resource_attrs = vec![
        string_attr("service.name", service_name),
        string_attr("actrail.trace.display_name", trace.display_name.as_str()),
        string_attr("actrail.trace.profile_name", trace.profile_name.as_str()),
        int_attr("actrail.trace.id", trace.trace_id.get()),
    ];
    // Emit the container the root agent ran in, when resolved. `container.id`
    // is the OpenTelemetry semantic convention for this value.
    if let Some(container_id) = trace.root_container_id.as_deref() {
        resource_attrs.push(string_attr("container.id", container_id));
    }
    if let Some(pod_uid) = trace.root_pod_uid.as_deref() {
        resource_attrs.push(string_attr("k8s.pod.uid", pod_uid));
    }
    if let Some(host_id) = trace.root_host_id.as_deref() {
        resource_attrs.push(string_attr("host.id", host_id));
    }
    format!(
        "{{\"resourceSpans\":[{{\"resource\":{{\"attributes\":[{}]}},\"scopeSpans\":[{{\"scope\":{{\"name\":\"actrail.semantic_actions\",\"version\":\"{}\"}},\"spans\":[{}]}}]}}]}}",
        resource_attrs.join(","),
        env!("CARGO_PKG_VERSION"),
        spans.join(",")
    )
}

fn render_span(trace_id: &str, action: &SemanticAction, links: &[SemanticActionLink]) -> String {
    let mut attrs = vec![
        string_attr("actrail.action.id", &action.action_id),
        string_attr("actrail.action.kind", action.kind.as_str()),
        string_attr("actrail.action.status", action.status.as_str()),
        string_attr("actrail.action.completeness", action.completeness.as_str()),
        int_attr("actrail.process.id", action.process.get()),
    ];
    for (key, value) in &action.attributes {
        attrs.push(string_attr(key, value));
    }

    let events = action
        .evidence
        .iter()
        .map(|evidence| {
            let attrs = [
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
    let parent = parent_link(action, links);
    let parent_span_id = parent
        .map(|link| {
            format!(
                ",\"parentSpanId\":{}",
                quoted(&otel_span_id(&link.parent_action_id))
            )
        })
        .unwrap_or_default();
    let span_links = support_links(action, links, parent)
        .map(|link| render_span_link(trace_id, link))
        .collect::<Vec<_>>();

    format!(
        "{{\"traceId\":{},\"spanId\":{}{},\"name\":{},\"kind\":\"{}\",\"startTimeUnixNano\":\"{}\",\"endTimeUnixNano\":\"{}\",\"attributes\":[{}],\"events\":[{}],\"links\":[{}],\"status\":{{\"code\":\"{}\"}}}}",
        quoted(trace_id),
        quoted(&otel_span_id(&action.action_id)),
        parent_span_id,
        quoted(&action.title),
        span_kind(action.kind),
        unix_nanos(action.start_time),
        unix_nanos(action.end_time.unwrap_or(action.start_time)),
        attrs.join(","),
        events.join(","),
        span_links.join(","),
        status_code(action.status)
    )
}

fn span_kind(kind: SemanticActionKind) -> &'static str {
    match kind {
        SemanticActionKind::HttpMessage
        | SemanticActionKind::LlmCall
        | SemanticActionKind::LlmRequest
        | SemanticActionKind::LlmResponse => "SPAN_KIND_CLIENT",
        SemanticActionKind::ProcessExec
        | SemanticActionKind::ProcessExit
        | SemanticActionKind::AgentIdentity
        | SemanticActionKind::AgentExit
        | SemanticActionKind::CommandInvocation
        | SemanticActionKind::ProcessForkAttempt
        | SemanticActionKind::AgentInvocation
        | SemanticActionKind::FileRead
        | SemanticActionKind::FileWrite
        | SemanticActionKind::FileModify
        | SemanticActionKind::FileTtyIo
        | SemanticActionKind::FileBulkRead
        | SemanticActionKind::FsEnumerate
        | SemanticActionKind::McpToolCall
        | SemanticActionKind::McpRequest
        | SemanticActionKind::McpResponse
        | SemanticActionKind::McpStdin
        | SemanticActionKind::McpStdout
        | SemanticActionKind::SseStream
        | SemanticActionKind::SseEvent
        | SemanticActionKind::EnforcementDecision => "SPAN_KIND_INTERNAL",
    }
}

pub(crate) fn parent_link<'a>(
    action: &SemanticAction,
    links: &'a [SemanticActionLink],
) -> Option<&'a SemanticActionLink> {
    links
        .iter()
        .filter(|link| !link_invalidated_by_child_parent_identity(action, link))
        .filter(|link| link.child_action_id == action.action_id && link_is_parent_child(link.role))
        .min_by_key(|link| parent_role_priority(link.role))
}

pub(crate) fn support_links<'a>(
    action: &SemanticAction,
    links: &'a [SemanticActionLink],
    parent: Option<&'a SemanticActionLink>,
) -> impl Iterator<Item = &'a SemanticActionLink> {
    links.iter().filter(move |link| {
        link.child_action_id == action.action_id
            && !link_invalidated_by_child_parent_identity(action, link)
            && !parent.is_some_and(|parent| {
                parent.parent_action_id == link.parent_action_id
                    && parent.child_action_id == link.child_action_id
                    && parent.role == link.role
            })
    })
}

fn link_invalidated_by_child_parent_identity(
    action: &SemanticAction,
    link: &SemanticActionLink,
) -> bool {
    if !link.valid {
        return true;
    }
    action
        .attributes
        .get(ATTR_PROCESS_PARENT_IDENTITY_STATE)
        .is_some_and(|state| state == PROCESS_PARENT_IDENTITY_STATE_CONFLICT)
        && matches!(
            link.role,
            SemanticActionLinkRole::AgentPerformedAction
                | SemanticActionLinkRole::CommandContainsCommandInvocation
                | SemanticActionLinkRole::CommandContainsMcpToolCall
        )
}

fn link_is_parent_child(role: SemanticActionLinkRole) -> bool {
    matches!(
        role,
        SemanticActionLinkRole::AgentPerformedAction
            | SemanticActionLinkRole::CommandContainsFileAccess
            | SemanticActionLinkRole::CommandContainsProcessForkAttempt
            | SemanticActionLinkRole::CommandContainsProcessExec
            | SemanticActionLinkRole::CommandContainsCommandInvocation
            | SemanticActionLinkRole::CommandContainsLlmCall
            | SemanticActionLinkRole::CommandContainsMcpToolCall
            | SemanticActionLinkRole::McpToolCallRequest
            | SemanticActionLinkRole::McpToolCallResponse
            | SemanticActionLinkRole::McpRequestStdout
            | SemanticActionLinkRole::McpResponseStdin
            | SemanticActionLinkRole::FileWriteContainsFileEvent
            | SemanticActionLinkRole::AgentInvocationExec
            | SemanticActionLinkRole::AgentInvocationChildLlmRequest
            | SemanticActionLinkRole::LlmCallRequest
            | SemanticActionLinkRole::LlmCallResponse
            | SemanticActionLinkRole::LlmResponseSseStream
            | SemanticActionLinkRole::SseStreamEvent
    )
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum ParentRolePriority {
    AgentInvocationExec,
    CommandContainsProcessExec,
    CommandContainsCommandInvocation,
    CommandContainsLlmCall,
    CommandContainsMcpToolCall,
    McpToolCallRequest,
    McpToolCallResponse,
    McpRequestStdout,
    McpResponseStdin,
    AgentPerformedAction,
    CommandContainsProcessForkAttempt,
    CommandContainsFileAccess,
    AgentInvocationChildLlmRequest,
    LlmCallRequest,
    LlmCallResponse,
    FileWriteContainsFileEvent,
    LlmResponseSseStream,
    SseStreamEvent,
    LlmRequestHttpMessage,
    LlmRequestLlmResponse,
    LlmResponseHttpMessage,
}

fn parent_role_priority(role: SemanticActionLinkRole) -> ParentRolePriority {
    match role {
        SemanticActionLinkRole::AgentInvocationExec => ParentRolePriority::AgentInvocationExec,
        SemanticActionLinkRole::CommandContainsProcessExec => {
            ParentRolePriority::CommandContainsProcessExec
        }
        SemanticActionLinkRole::CommandContainsCommandInvocation => {
            ParentRolePriority::CommandContainsCommandInvocation
        }
        SemanticActionLinkRole::CommandContainsLlmCall => {
            ParentRolePriority::CommandContainsLlmCall
        }
        SemanticActionLinkRole::CommandContainsMcpToolCall => {
            ParentRolePriority::CommandContainsMcpToolCall
        }
        SemanticActionLinkRole::McpToolCallRequest => ParentRolePriority::McpToolCallRequest,
        SemanticActionLinkRole::McpToolCallResponse => ParentRolePriority::McpToolCallResponse,
        SemanticActionLinkRole::McpRequestStdout => ParentRolePriority::McpRequestStdout,
        SemanticActionLinkRole::McpResponseStdin => ParentRolePriority::McpResponseStdin,
        SemanticActionLinkRole::CommandContainsProcessForkAttempt => {
            ParentRolePriority::CommandContainsProcessForkAttempt
        }
        SemanticActionLinkRole::CommandContainsFileAccess => {
            ParentRolePriority::CommandContainsFileAccess
        }
        SemanticActionLinkRole::AgentPerformedAction => ParentRolePriority::AgentPerformedAction,
        SemanticActionLinkRole::AgentInvocationChildLlmRequest => {
            ParentRolePriority::AgentInvocationChildLlmRequest
        }
        SemanticActionLinkRole::LlmCallRequest => ParentRolePriority::LlmCallRequest,
        SemanticActionLinkRole::LlmCallResponse => ParentRolePriority::LlmCallResponse,
        SemanticActionLinkRole::FileWriteContainsFileEvent => {
            ParentRolePriority::FileWriteContainsFileEvent
        }
        SemanticActionLinkRole::LlmResponseSseStream => ParentRolePriority::LlmResponseSseStream,
        SemanticActionLinkRole::SseStreamEvent => ParentRolePriority::SseStreamEvent,
        SemanticActionLinkRole::LlmRequestHttpMessage => ParentRolePriority::LlmRequestHttpMessage,
        SemanticActionLinkRole::LlmRequestLlmResponse => ParentRolePriority::LlmRequestLlmResponse,
        SemanticActionLinkRole::LlmResponseHttpMessage => {
            ParentRolePriority::LlmResponseHttpMessage
        }
    }
}

fn render_span_link(trace_id: &str, link: &SemanticActionLink) -> String {
    let attrs = [
        string_attr("actrail.link.role", link.role.as_str()),
        string_attr("actrail.link.confidence", link.confidence.as_str()),
    ];
    format!(
        "{{\"traceId\":{},\"spanId\":{},\"attributes\":[{}]}}",
        quoted(trace_id),
        quoted(&otel_span_id(&link.parent_action_id)),
        attrs.join(",")
    )
}

fn status_code(status: SemanticActionStatus) -> &'static str {
    match status {
        SemanticActionStatus::Success => "STATUS_CODE_OK",
        SemanticActionStatus::Error => "STATUS_CODE_ERROR",
        SemanticActionStatus::InProgress | SemanticActionStatus::Unknown => "STATUS_CODE_UNSET",
    }
}

/// Render the persistent external trace identity as fixed-width OTLP hex.
fn otel_trace_id(trace: &TraceRecord) -> String {
    format!("{:032x}", otel_trace_id_u128(trace))
}

/// The persistent 128-bit identity shared by JSON and protobuf encoders.
pub(crate) fn otel_trace_id_u128(trace: &TraceRecord) -> u128 {
    u128::from_be_bytes(*trace.otel_trace_id.as_bytes())
}

fn otel_span_id(action_id: &str) -> String {
    format!("{:016x}", otel_span_id_u64(action_id))
}

/// The 64-bit span id as a number (shared by the JSON and protobuf encoders).
pub(crate) fn otel_span_id_u64(action_id: &str) -> u64 {
    stable_hash(action_id.as_bytes())
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

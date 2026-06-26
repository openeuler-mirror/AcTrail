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
const ATTR_ACTION_VALID: &str = "actrail.action.valid";
const ACTION_VALID_FALSE: &str = "false";
const ATTR_LINK_VALID: &str = "actrail.link.valid";
const LINK_VALID_FALSE: &str = "false";

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
    let mut spans = Vec::new();
    for action in actions {
        if action_invalidated(action) {
            continue;
        }
        spans.push(render_span(trace, action, links));
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
    format!(
        "{{\"resourceSpans\":[{{\"resource\":{{\"attributes\":[{}]}},\"scopeSpans\":[{{\"scope\":{{\"name\":\"actrail.semantic_actions\",\"version\":\"{}\"}},\"spans\":[{}]}}]}}]}}",
        resource_attrs.join(","),
        env!("CARGO_PKG_VERSION"),
        spans.join(",")
    )
}

fn render_span(
    trace: &TraceRecord,
    action: &SemanticAction,
    links: &[SemanticActionLink],
) -> String {
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
        .map(|link| render_span_link(link))
        .collect::<Vec<_>>();

    format!(
        "{{\"traceId\":{},\"spanId\":{}{},\"name\":{},\"kind\":\"{}\",\"startTimeUnixNano\":\"{}\",\"endTimeUnixNano\":\"{}\",\"attributes\":[{}],\"events\":[{}],\"links\":[{}],\"status\":{{\"code\":\"{}\"}}}}",
        quoted(&otel_trace_id(trace)),
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
        | SemanticActionKind::CommandInvocation
        | SemanticActionKind::ProcessForkAttempt
        | SemanticActionKind::AgentInvocation
        | SemanticActionKind::FileRead
        | SemanticActionKind::FileWrite
        | SemanticActionKind::FileModify
        | SemanticActionKind::FileTtyIo
        | SemanticActionKind::FileBulkRead
        | SemanticActionKind::FsEnumerate
        | SemanticActionKind::SseStream
        | SemanticActionKind::SseEvent
        | SemanticActionKind::EnforcementDecision => "SPAN_KIND_INTERNAL",
    }
}

fn parent_link<'a>(
    action: &SemanticAction,
    links: &'a [SemanticActionLink],
) -> Option<&'a SemanticActionLink> {
    links
        .iter()
        .filter(|link| !link_invalidated_by_child_parent_identity(action, link))
        .filter(|link| link.child_action_id == action.action_id && link_is_parent_child(link.role))
        .min_by_key(|link| parent_role_priority(link.role))
}

fn support_links<'a>(
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

fn action_invalidated(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(ATTR_ACTION_VALID)
        .is_some_and(|value| value == ACTION_VALID_FALSE)
}

fn link_invalidated_by_child_parent_identity(
    action: &SemanticAction,
    link: &SemanticActionLink,
) -> bool {
    if link
        .attributes
        .get(ATTR_LINK_VALID)
        .is_some_and(|value| value == LINK_VALID_FALSE)
    {
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

fn render_span_link(link: &SemanticActionLink) -> String {
    let attrs = vec![
        string_attr("actrail.link.role", link.role.as_str()),
        string_attr("actrail.link.confidence", link.confidence.as_str()),
    ];
    format!(
        "{{\"traceId\":{},\"spanId\":{},\"attributes\":[{}]}}",
        quoted(&format!("{:032x}", link.trace_id.get())),
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

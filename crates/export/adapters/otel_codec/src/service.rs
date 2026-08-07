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
    let trace_id = otel_trace_id(trace);
    let mut spans = Vec::new();
    for action in actions {
        if action_invalidated(action) {
            continue;
        }
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

pub(crate) fn action_invalidated(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(ATTR_ACTION_VALID)
        .is_some_and(|value| value == ACTION_VALID_FALSE)
}

fn link_invalidated_by_child_parent_identity(
    action: &SemanticAction,
    link: &SemanticActionLink,
) -> bool {
    if !link.valid
        || link
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

/// Render the daemon-local trace id as the fixed-width OTLP 128-bit value.
fn otel_trace_id(trace: &TraceRecord) -> String {
    format!("{:032x}", otel_trace_id_u128(trace))
}

/// The 128-bit trace id as a number, so the JSON (hex) and protobuf (16 bytes)
/// encoders derive an identical value from one place.
///
/// `TraceId` is a per-daemon counter, so two daemons reporting into one shared
/// Collector still emit colliding ids. Widening this into a globally unique id
/// needs a scope anchor that survives a reload from SQLite — `root_host_id`
/// and `root_pod_uid` do not (`sqlite/records/rows.rs`), and anchoring on them
/// would give one trace two different ids depending on the export path. That
/// scoping is deferred to the centralized-Collector work, which settles the
/// anchor and its schema together.
pub(crate) fn otel_trace_id_u128(trace: &TraceRecord) -> u128 {
    u128::from(trace.trace_id.get())
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

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use model_core::ids::{ProfileName, TraceId, TraceName};
    use model_core::process::ProcessIdentity;
    use model_core::trace::{TraceAlertToken, TraceRecord};

    use super::{otel_trace_id, render_otlp_json};

    fn trace() -> TraceRecord {
        TraceRecord::new(
            TraceId::new(1),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(100),
            TraceName::new("test trace"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        )
    }

    #[test]
    fn resource_emits_runtime_identity_when_resolved() {
        let mut record = trace();
        record.root_container_id = Some("6bfb54c1b8d9".to_string());
        record.root_pod_uid = Some("2ee7d8a2-e832-4a13-b26c-02ad9ae4a8f6".to_string());
        record.root_host_id = Some("4C4C4544-0042-1234-8000-abcdef012345".to_string());

        let json = render_otlp_json(&record, &[], &[]).expect("render");

        for key in ["container.id", "k8s.pod.uid", "host.id"] {
            assert!(json.contains(&format!("\"{key}\"")), "missing {key}");
        }
    }

    /// The same trace must export one id on every path. `root_host_id` and
    /// `root_pod_uid` live only in `TraceRuntime`; a record reloaded from
    /// SQLite carries `None` for both (see `sqlite/records/rows.rs`), so any
    /// id derived from them would split the live export and the
    /// `actrailviewer` storage export of one trace into two OTLP traces.
    #[test]
    fn trace_id_does_not_depend_on_unpersisted_runtime_identity() {
        let live = {
            let mut record = trace();
            record.root_host_id = Some("4C4C4544-0042-1234-8000-abcdef012345".to_string());
            record.root_pod_uid = Some("2ee7d8a2-e832-4a13-b26c-02ad9ae4a8f6".to_string());
            record.root_container_id = Some("6bfb54c1b8d9".to_string());
            record
        };
        // What `trace_from_row` reconstructs for the same trace.
        let reloaded = {
            let mut record = trace();
            record.root_container_id = Some("6bfb54c1b8d9".to_string());
            record
        };
        let host_rooted = trace();

        assert_eq!(otel_trace_id(&live), otel_trace_id(&reloaded));
        assert_eq!(otel_trace_id(&live), otel_trace_id(&host_rooted));
    }
}

//! Semantic action builders shared by live projectors.

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind, attr_keys as attrs, evidence_roles,
};

pub(super) const ATTR_AGENT_IDENTITY_STATUS: &str = attrs::agent::IDENTITY_STATUS;
pub(super) const ATTR_AGENT_IDENTITY_SOURCE: &str = attrs::agent::IDENTITY_SOURCE;
pub(super) const ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID: &str =
    attrs::agent::IDENTITY_EVIDENCE_ACTION_ID;
pub(super) const ATTR_AGENT_INVOCATION_TRIGGER: &str = attrs::agent_invocation::TRIGGER;
pub(super) const ATTR_AGENT_INVOCATION_EVIDENCE_ACTION_ID: &str =
    attrs::agent_invocation::EVIDENCE_ACTION_ID;
pub(super) const ATTR_PROCESS_PARENT_GENERATION: &str = attrs::process_parent::GENERATION;
pub(super) const ATTR_PROCESS_PARENT_IDENTITY_STATE: &str = attrs::process_parent::IDENTITY_STATE;
pub(super) const ATTR_PROCESS_PARENT_PID: &str = attrs::process_parent::PID;
pub(super) const ATTR_PROCESS_PARENT_PID_NAMESPACE: &str = attrs::process_parent::PID_NAMESPACE;
pub(super) const ATTR_PROCESS_PARENT_START_TIME_TICKS: &str =
    attrs::process_parent::START_TIME_TICKS;
pub(super) const ATTR_PROCESS_PARENT_TASK_ID: &str = attrs::process_parent::TASK_ID;
pub(super) const PROCESS_PARENT_IDENTITY_STATE_CONFLICT: &str = "conflict";
pub(super) const PROCESS_PARENT_IDENTITY_STATE_OBSERVED: &str = "observed";

pub(super) fn action_for_live_state(action: &SemanticAction) -> SemanticAction {
    let mut state_action = action.clone();
    for key in LIVE_STATE_OMITTED_ATTRIBUTES {
        state_action.attributes.remove(*key);
    }
    state_action
}

const LIVE_STATE_OMITTED_ATTRIBUTES: &[&str] = &[
    attrs::http_request::BODY_JSON,
    attrs::http_request::BODY_TEXT,
    attrs::http_request::HEADERS_HPACK_BASE64,
    attrs::http_request::HEADERS_TEXT,
    attrs::http_response::BODY_JSON,
    attrs::http_response::BODY_TEXT,
    attrs::http_response::HEADERS_HPACK_BASE64,
    attrs::http_response::HEADERS_TEXT,
    attrs::llm_request::BODY_JSON,
    attrs::llm_request::BODY_TEXT,
    attrs::llm_request::PAYLOAD_TEXT,
    attrs::llm_response::CONTENT_TEXT,
    attrs::llm_response::OUTPUT_TEXT,
    attrs::llm_response::PAYLOAD_TEXT,
    attrs::llm_response::REASONING_TEXT,
    attrs::llm_response::SSE_EVENTS_JSON,
    attrs::llm_response::TOOL_CALLS_JSON,
];

pub(super) fn process_exec_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Process(payload) = &event.payload else {
        unreachable!("process_exec_action only receives process events")
    };
    let mut attributes = payload.metadata.clone();
    if let Some(executable) = &payload.executable {
        attributes.insert(attrs::process::EXECUTABLE.to_string(), executable.clone());
    }
    if let Some(parent) = &payload.parent {
        insert_parent_identity_attributes(&mut attributes, parent);
    }
    SemanticAction {
        action_id: process_action_id(event.envelope.trace_id, &event.envelope.process, "exec"),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::ProcessExec,
        title: payload
            .executable
            .clone()
            .unwrap_or_else(|| format!("exec pid {}", event.envelope.process.pid)),
        start_time: event.envelope.observed_at,
        end_time: None,
        process: event.envelope.process.clone(),
        status: SemanticActionStatus::InProgress,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, evidence_roles::process::EXEC)],
    }
}

pub(super) fn process_fork_attempt_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Process(payload) = &event.payload else {
        unreachable!("process_fork_attempt_action only receives process events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert(
        attrs::process::OPERATION.to_string(),
        payload.operation.clone(),
    );
    SemanticAction {
        action_id: event_action_id(event, SemanticActionKind::ProcessForkAttempt.as_str()),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::ProcessForkAttempt,
        title: attributes
            .get("syscall")
            .cloned()
            .unwrap_or_else(|| "fork attempt".to_string()),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, evidence_roles::process::FORK_ATTEMPT)],
    }
}

pub(super) fn process_exit_status(exit_code: Option<&String>) -> SemanticActionStatus {
    match exit_code.and_then(|value| value.parse::<i32>().ok()) {
        Some(0) | None => SemanticActionStatus::Success,
        Some(_) => SemanticActionStatus::Error,
    }
}

pub(super) fn file_modify_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::File(payload) = &event.payload else {
        unreachable!("file_modify_action only receives file events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert(
        attrs::file::OPERATION.to_string(),
        payload.operation.clone(),
    );
    if let Some(path) = &payload.path {
        attributes.insert(attrs::file::PATH.to_string(), path.clone());
    }
    if let Some(result) = payload.result {
        attributes.insert(attrs::syscall::RESULT.to_string(), result.to_string());
    }
    SemanticAction {
        action_id: event_action_id(event, SemanticActionKind::FileModify.as_str()),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::FileModify,
        title: payload
            .path
            .clone()
            .unwrap_or_else(|| format!("file {}", payload.operation)),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status: status_from_result(payload.result),
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(
            event,
            SemanticActionKind::FileModify.as_str(),
        )],
    }
}

pub(super) fn http_message_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Application(payload) = &event.payload else {
        unreachable!("http_message_action only receives application events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert(
        attrs::network::PROTOCOL_NAME.to_string(),
        "http".to_string(),
    );
    attributes.insert(
        attrs::network::PROTOCOL_VERSION.to_string(),
        payload.protocol.clone(),
    );
    attributes.insert(
        attrs::http::OPERATION.to_string(),
        payload.operation.clone(),
    );
    SemanticAction {
        action_id: event_action_id(event, SemanticActionKind::HttpMessage.as_str()),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::HttpMessage,
        title: payload.summary.clone(),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(
            event,
            SemanticActionKind::HttpMessage.as_str(),
        )],
    }
}

pub(super) fn enforcement_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Enforcement(payload) = &event.payload else {
        unreachable!("enforcement_action only receives enforcement events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert(
        attrs::enforcement::BACKEND.to_string(),
        payload.backend.clone(),
    );
    attributes.insert(
        attrs::enforcement::OPERATION.to_string(),
        payload.operation.clone(),
    );
    attributes.insert(
        attrs::enforcement::DECISION.to_string(),
        payload.decision.clone(),
    );
    attributes.insert(
        attrs::enforcement::RESULT.to_string(),
        payload.result.clone(),
    );
    if let Some(path) = &payload.path {
        attributes.insert(attrs::file::PATH.to_string(), path.clone());
    }
    if let Some(rule_id) = &payload.rule_id {
        attributes.insert(attrs::enforcement::RULE_ID.to_string(), rule_id.clone());
    }
    SemanticAction {
        action_id: event_action_id(event, SemanticActionKind::EnforcementDecision.as_str()),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::EnforcementDecision,
        title: format!("{} {}", payload.decision, payload.operation),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status: enforcement_status(&payload.result),
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(
            event,
            SemanticActionKind::EnforcementDecision.as_str(),
        )],
    }
}

pub(super) fn is_http_protocol(protocol: &str) -> bool {
    let protocol = protocol.to_ascii_lowercase();
    protocol == "h2"
        || protocol == "http2"
        || protocol == "http/2"
        || protocol == "http/2.0"
        || protocol.starts_with("http/")
}

pub(super) fn is_file_modify_operation(operation: &str) -> bool {
    matches!(
        operation,
        "write" | "writev" | "truncate" | "unlink" | "rename" | "mkdir" | "rmdir" | "mmap_shared"
    )
}

pub(super) fn event_evidence(event: &DomainEvent, role: &str) -> SemanticEvidence {
    SemanticEvidence {
        kind: SemanticEvidenceKind::Event,
        id: event.envelope.event_id.get(),
        role: role.to_string(),
    }
}

pub(super) fn append_missing_evidence(
    target: &mut Vec<SemanticEvidence>,
    source: &[SemanticEvidence],
) {
    for evidence in source {
        if !target.contains(evidence) {
            target.push(evidence.clone());
        }
    }
}

pub(super) fn insert_parent_identity_attributes(
    attributes: &mut std::collections::BTreeMap<String, String>,
    parent: &ProcessIdentity,
) {
    attributes.insert(ATTR_PROCESS_PARENT_PID.to_string(), parent.pid.to_string());
    if let Some(task_id) = parent.task_id {
        attributes.insert(ATTR_PROCESS_PARENT_TASK_ID.to_string(), task_id.to_string());
    }
    attributes.insert(
        ATTR_PROCESS_PARENT_START_TIME_TICKS.to_string(),
        parent.start_time_ticks.to_string(),
    );
    if let Some(pid_namespace) = &parent.pid_namespace {
        attributes.insert(
            ATTR_PROCESS_PARENT_PID_NAMESPACE.to_string(),
            pid_namespace.as_str().to_string(),
        );
    }
    attributes.insert(
        ATTR_PROCESS_PARENT_GENERATION.to_string(),
        parent.generation.to_string(),
    );
    attributes.insert(
        ATTR_PROCESS_PARENT_IDENTITY_STATE.to_string(),
        PROCESS_PARENT_IDENTITY_STATE_OBSERVED.to_string(),
    );
}

pub(super) fn event_action_id(event: &DomainEvent, suffix: &str) -> String {
    event_action_id_for_event_id(event.envelope.trace_id, event.envelope.event_id, suffix)
}

pub(super) fn event_action_id_for_event_id(
    trace_id: TraceId,
    event_id: model_core::ids::EventId,
    suffix: &str,
) -> String {
    format!(
        "trace:{}:event:{}:{}",
        trace_id.get(),
        event_id.get(),
        suffix
    )
}

pub(super) fn process_action_id(
    trace_id: TraceId,
    process: &ProcessIdentity,
    suffix: &str,
) -> String {
    format!(
        "trace:{}:process:{}:{}:{}",
        trace_id.get(),
        process.pid,
        process.generation,
        suffix
    )
}

pub(super) fn llm_call_action_id_from_request_action_id(request_action_id: &str) -> String {
    request_action_id
        .strip_suffix(":llm.request")
        .map(|prefix| format!("{prefix}:llm.call"))
        .unwrap_or_else(|| format!("{request_action_id}:llm.call"))
}

pub(super) fn status_from_result(result: Option<i32>) -> SemanticActionStatus {
    match result {
        Some(value) if value < 0 => SemanticActionStatus::Error,
        Some(_) => SemanticActionStatus::Success,
        None => SemanticActionStatus::Unknown,
    }
}

fn enforcement_status(result: &str) -> SemanticActionStatus {
    match result {
        "allowed" | "allow" | "success" => SemanticActionStatus::Success,
        "denied" | "deny" | "blocked" | "error" => SemanticActionStatus::Error,
        _ => SemanticActionStatus::Unknown,
    }
}

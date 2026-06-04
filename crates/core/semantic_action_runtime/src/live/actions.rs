//! Semantic action builders shared by live projectors.

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind,
};

pub(super) const ATTR_AGENT_IDENTITY_STATUS: &str = "agent.identity.status";
pub(super) const ATTR_AGENT_IDENTITY_SOURCE: &str = "agent.identity.source";
pub(super) const ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID: &str = "agent.identity.evidence_action_id";
pub(super) const ATTR_AGENT_CANDIDATE_COMMAND_MATCH: &str =
    "agent.identity.candidate_command_match";
pub(super) const ATTR_AGENT_CANDIDATE_COMMAND: &str = "agent.identity.candidate_command";
pub(super) const ATTR_AGENT_INVOCATION_TRIGGER: &str = "agent.invocation.trigger";
pub(super) const ATTR_AGENT_INVOCATION_EVIDENCE_ACTION_ID: &str =
    "agent.invocation.evidence_action_id";

pub(super) fn process_exec_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Process(payload) = &event.payload else {
        unreachable!("process_exec_action only receives process events")
    };
    let mut attributes = payload.metadata.clone();
    if let Some(executable) = &payload.executable {
        attributes.insert("process.executable".to_string(), executable.clone());
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
        evidence: vec![event_evidence(event, "process.exec")],
    }
}

pub(super) fn process_fork_attempt_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Process(payload) = &event.payload else {
        unreachable!("process_fork_attempt_action only receives process events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert("process.operation".to_string(), payload.operation.clone());
    SemanticAction {
        action_id: event_action_id(event, "process.fork_attempt"),
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
        evidence: vec![event_evidence(event, "process.fork_attempt")],
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
    attributes.insert("file.operation".to_string(), payload.operation.clone());
    if let Some(path) = &payload.path {
        attributes.insert("file.path".to_string(), path.clone());
    }
    if let Some(result) = payload.result {
        attributes.insert("syscall.result".to_string(), result.to_string());
    }
    SemanticAction {
        action_id: event_action_id(event, "file.modify"),
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
        evidence: vec![event_evidence(event, "file.modify")],
    }
}

pub(super) fn http_message_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Application(payload) = &event.payload else {
        unreachable!("http_message_action only receives application events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert("network.protocol.name".to_string(), "http".to_string());
    attributes.insert(
        "network.protocol.version".to_string(),
        payload.protocol.clone(),
    );
    attributes.insert("http.operation".to_string(), payload.operation.clone());
    SemanticAction {
        action_id: event_action_id(event, "http.message"),
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
        evidence: vec![event_evidence(event, "http.message")],
    }
}

pub(super) fn enforcement_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Enforcement(payload) = &event.payload else {
        unreachable!("enforcement_action only receives enforcement events")
    };
    let mut attributes = payload.metadata.clone();
    attributes.insert("enforcement.backend".to_string(), payload.backend.clone());
    attributes.insert(
        "enforcement.operation".to_string(),
        payload.operation.clone(),
    );
    attributes.insert("enforcement.decision".to_string(), payload.decision.clone());
    attributes.insert("enforcement.result".to_string(), payload.result.clone());
    if let Some(path) = &payload.path {
        attributes.insert("file.path".to_string(), path.clone());
    }
    if let Some(rule_id) = &payload.rule_id {
        attributes.insert("enforcement.rule_id".to_string(), rule_id.clone());
    }
    SemanticAction {
        action_id: event_action_id(event, "enforcement.decision"),
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
        evidence: vec![event_evidence(event, "enforcement.decision")],
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

pub(super) fn event_action_id(event: &DomainEvent, suffix: &str) -> String {
    format!(
        "trace:{}:event:{}:{}",
        event.envelope.trace_id.get(),
        event.envelope.event_id.get(),
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

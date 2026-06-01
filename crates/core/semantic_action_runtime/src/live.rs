//! Observation-time semantic action projection.

use std::collections::BTreeMap;

use config_core::daemon::AgentInvocationConfig;
use model_core::event::{DomainEvent, EventPayload};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind,
};

#[derive(Default)]
pub struct LiveSemanticActionRuntime {
    process_execs: BTreeMap<ProcessIdentity, SemanticAction>,
    agent_ancestry_by_pid: BTreeMap<u32, AgentProcess>,
    agent_invocation_enabled: bool,
    agent_commands: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentProcess {
    pid: u32,
    executable: String,
    command_line: Option<String>,
}

impl LiveSemanticActionRuntime {
    pub fn new(config: AgentInvocationConfig) -> Self {
        Self {
            process_execs: BTreeMap::new(),
            agent_ancestry_by_pid: BTreeMap::new(),
            agent_invocation_enabled: config.enabled,
            agent_commands: config.commands,
        }
    }

    pub fn observe_event(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        match &event.payload {
            EventPayload::Process(payload) if payload.operation == "exec" => {
                let action = process_exec_action(event);
                let action = self.merge_process_exec(action);
                let mut actions = vec![action.clone()];
                if let Some(agent_action) = self.observe_agent_exec(event, &action) {
                    actions.push(agent_action);
                }
                actions
            }
            EventPayload::Process(payload) if payload.operation == "fork_attempt" => {
                vec![process_fork_attempt_action(event)]
            }
            EventPayload::Process(payload) if payload.operation == "exit" => self
                .process_execs
                .remove(&event.envelope.process)
                .map(|mut action| {
                    self.agent_ancestry_by_pid
                        .remove(&event.envelope.process.pid);
                    action.end_time = Some(event.envelope.observed_at);
                    action.status = process_exit_status(payload.metadata.get("exit_code"));
                    action.evidence.push(event_evidence(event, "process.exit"));
                    action
                })
                .into_iter()
                .collect(),
            EventPayload::File(payload) if is_file_modify_operation(&payload.operation) => {
                vec![file_modify_action(event)]
            }
            EventPayload::Application(payload) if is_http_protocol(&payload.protocol) => {
                vec![http_message_action(event)]
            }
            EventPayload::Enforcement(_) => vec![enforcement_action(event)],
            _ => Vec::new(),
        }
    }

    fn merge_process_exec(&mut self, mut action: SemanticAction) -> SemanticAction {
        if let Some(existing) = self.process_execs.get(&action.process) {
            for (key, value) in &existing.attributes {
                action
                    .attributes
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            if existing
                .attributes
                .get("seccomp_observed")
                .is_some_and(|value| value == "true")
            {
                action.title = existing.title.clone();
            }
        }
        self.process_execs
            .insert(action.process.clone(), action.clone());
        action
    }

    fn observe_agent_exec(
        &mut self,
        event: &DomainEvent,
        exec_action: &SemanticAction,
    ) -> Option<SemanticAction> {
        if !self.agent_invocation_enabled {
            return None;
        }
        let EventPayload::Process(payload) = &event.payload else {
            return None;
        };
        let parent_pid = payload
            .metadata
            .get("ppid")
            .or_else(|| payload.metadata.get("stat_ppid"))
            .and_then(|value| value.parse::<u32>().ok())
            .or_else(|| payload.parent.as_ref().map(|parent| parent.pid));
        let parent_agent = parent_pid.and_then(|pid| self.agent_ancestry_by_pid.get(&pid).cloned());
        let child_agent = agent_process(&self.agent_commands, exec_action);
        self.update_agent_ancestry(
            event.envelope.process.pid,
            parent_agent.as_ref(),
            child_agent.as_ref(),
        );
        let child_agent = child_agent?;
        let parent_agent = parent_agent?;
        if parent_agent.pid == event.envelope.process.pid {
            return None;
        }
        let mut attributes = BTreeMap::new();
        attributes.insert(
            "agent.child.pid".to_string(),
            event.envelope.process.pid.to_string(),
        );
        attributes.insert("agent.parent.pid".to_string(), parent_agent.pid.to_string());
        attributes.insert(
            "agent.parent.executable".to_string(),
            parent_agent.executable.clone(),
        );
        if let Some(command_line) = &parent_agent.command_line {
            attributes.insert(
                "agent.parent.command_line".to_string(),
                command_line.clone(),
            );
        }
        attributes.insert(
            "agent.child.executable".to_string(),
            child_agent.executable.clone(),
        );
        if let Some(command_line) = &child_agent.command_line {
            attributes.insert("agent.child.command_line".to_string(), command_line.clone());
        }
        Some(SemanticAction {
            action_id: event_action_id(event, "agent.invocation"),
            trace_id: event.envelope.trace_id,
            kind: SemanticActionKind::AgentInvocation,
            title: exec_action.title.clone(),
            start_time: event.envelope.observed_at,
            end_time: Some(event.envelope.observed_at),
            process: event.envelope.process.clone(),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence: vec![event_evidence(event, "agent.invocation")],
        })
    }

    fn update_agent_ancestry(
        &mut self,
        pid: u32,
        parent_agent: Option<&AgentProcess>,
        child_agent: Option<&AgentProcess>,
    ) {
        if let Some(child_agent) = child_agent {
            self.agent_ancestry_by_pid.insert(pid, child_agent.clone());
        } else if let Some(parent_agent) = parent_agent {
            self.agent_ancestry_by_pid.insert(pid, parent_agent.clone());
        }
    }
}

fn process_exec_action(event: &DomainEvent) -> SemanticAction {
    let EventPayload::Process(payload) = &event.payload else {
        unreachable!("process_exec_action only receives process events")
    };
    let mut attributes = payload.metadata.clone();
    if let Some(executable) = &payload.executable {
        attributes.insert("process.executable".to_string(), executable.clone());
    }
    SemanticAction {
        action_id: process_action_id(event, "exec"),
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

fn process_fork_attempt_action(event: &DomainEvent) -> SemanticAction {
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

fn file_modify_action(event: &DomainEvent) -> SemanticAction {
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

fn http_message_action(event: &DomainEvent) -> SemanticAction {
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

fn enforcement_action(event: &DomainEvent) -> SemanticAction {
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

fn process_exit_status(exit_code: Option<&String>) -> SemanticActionStatus {
    match exit_code.and_then(|value| value.parse::<i32>().ok()) {
        Some(0) | None => SemanticActionStatus::Success,
        Some(_) => SemanticActionStatus::Error,
    }
}

fn status_from_result(result: Option<i32>) -> SemanticActionStatus {
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

fn is_http_protocol(protocol: &str) -> bool {
    let protocol = protocol.to_ascii_lowercase();
    protocol == "h2"
        || protocol == "http2"
        || protocol == "http/2"
        || protocol == "http/2.0"
        || protocol.starts_with("http/")
}

fn is_file_modify_operation(operation: &str) -> bool {
    matches!(
        operation,
        "write" | "writev" | "truncate" | "unlink" | "rename" | "mkdir" | "rmdir" | "mmap_shared"
    )
}

fn agent_process(commands: &[String], action: &SemanticAction) -> Option<AgentProcess> {
    let executable = action
        .attributes
        .get("process.executable")
        .or_else(|| action.attributes.get("executable"))
        .map(|value| value.as_str())
        .unwrap_or(action.title.as_str())
        .to_string();
    let basename = executable.rsplit('/').next().unwrap_or(executable.as_str());
    if commands
        .iter()
        .any(|command| command == &executable || command == basename)
    {
        Some(AgentProcess {
            pid: action.process.pid,
            executable,
            command_line: action.attributes.get("command_line").cloned(),
        })
    } else {
        None
    }
}

fn event_evidence(event: &DomainEvent, role: &str) -> SemanticEvidence {
    SemanticEvidence {
        kind: SemanticEvidenceKind::Event,
        id: event.envelope.event_id.get(),
        role: role.to_string(),
    }
}

pub(crate) fn event_action_id(event: &DomainEvent, suffix: &str) -> String {
    format!(
        "trace:{}:event:{}:{}",
        event.envelope.trace_id.get(),
        event.envelope.event_id.get(),
        suffix
    )
}

fn process_action_id(event: &DomainEvent, suffix: &str) -> String {
    format!(
        "trace:{}:process:{}:{}:{}",
        event.envelope.trace_id.get(),
        event.envelope.process.pid,
        event.envelope.process.generation,
        suffix
    )
}

//! Agent identity and direct invocation projection.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence,
};

use super::actions::{
    ATTR_AGENT_CANDIDATE_COMMAND, ATTR_AGENT_CANDIDATE_COMMAND_MATCH,
    ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID, ATTR_AGENT_IDENTITY_SOURCE, ATTR_AGENT_IDENTITY_STATUS,
    ATTR_AGENT_INVOCATION_EVIDENCE_ACTION_ID, ATTR_AGENT_INVOCATION_TRIGGER, event_evidence,
    process_action_id, process_exit_status,
};

const AGENT_IDENTITY_STATUS_OBSERVED: &str = "observed";
const AGENT_IDENTITY_SOURCE_LLM_REQUEST: &str = "llm.request";
const AGENT_INVOCATION_TRIGGER_CHILD_LLM_REQUEST: &str = "child_llm_request";
const AGENT_PARENT_STATUS_UNKNOWN: &str = "unknown";
const AGENT_PARENT_STATUS_OBSERVED: &str = "observed";

pub(super) struct AgentProjector {
    process_execs: BTreeMap<ProcessIdentity, SemanticAction>,
    processes_by_pid: BTreeMap<(TraceId, u32), ProcessIdentity>,
    direct_edges: BTreeMap<ProcessIdentity, DirectProcessEdge>,
    llm_agents: BTreeMap<ProcessIdentity, AgentEvidence>,
    emitted_invocations: BTreeSet<AgentInvocationKey>,
    invocation_enabled: bool,
    commands: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentEvidence {
    trace_id: TraceId,
    action_id: String,
    observed_at: SystemTime,
    evidence: Vec<SemanticEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectProcessEdge {
    trace_id: TraceId,
    child: ProcessIdentity,
    parent: Option<ProcessIdentity>,
    parent_pid: Option<u32>,
    observed_at: SystemTime,
    exec_event_id: EventId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AgentInvocationKey {
    trace_id: TraceId,
    parent: Option<ProcessIdentity>,
    parent_pid: Option<u32>,
    child: ProcessIdentity,
}

impl AgentProjector {
    pub(super) fn new(invocation_enabled: bool, commands: Vec<String>) -> Self {
        Self {
            process_execs: BTreeMap::new(),
            processes_by_pid: BTreeMap::new(),
            direct_edges: BTreeMap::new(),
            llm_agents: BTreeMap::new(),
            emitted_invocations: BTreeSet::new(),
            invocation_enabled,
            commands,
        }
    }

    pub(super) fn observe_process_exec(
        &mut self,
        event: &DomainEvent,
        action: SemanticAction,
    ) -> Vec<SemanticAction> {
        let mut action = self.merge_process_exec(action);
        self.add_candidate_hint(&mut action);
        if let Some(evidence) = self.llm_agents.get(&action.process) {
            apply_agent_identity(&mut action, evidence);
        }
        self.process_execs
            .insert(action.process.clone(), action.clone());
        self.processes_by_pid.insert(
            (action.trace_id, action.process.pid),
            action.process.clone(),
        );
        if let Some(edge) = self.direct_edge(event) {
            self.direct_edges.insert(edge.child.clone(), edge);
        }

        let mut actions = vec![action.clone()];
        if let Some(invocation) = self.try_agent_invocation(&action.process) {
            actions.push(invocation);
        }
        actions
    }

    pub(super) fn observe_process_exit(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        let EventPayload::Process(payload) = &event.payload else {
            return Vec::new();
        };
        self.process_execs
            .get(&event.envelope.process)
            .cloned()
            .map(|mut action| {
                action.end_time = Some(event.envelope.observed_at);
                action.status = process_exit_status(payload.metadata.get("exit_code"));
                action.evidence.push(event_evidence(event, "process.exit"));
                self.process_execs
                    .insert(event.envelope.process.clone(), action.clone());
                action
            })
            .into_iter()
            .collect()
    }

    pub(super) fn observe_llm_request(&mut self, action: &SemanticAction) -> Vec<SemanticAction> {
        if action.kind != SemanticActionKind::LlmRequest
            || action.status == SemanticActionStatus::Error
        {
            return Vec::new();
        }
        let evidence = AgentEvidence {
            trace_id: action.trace_id,
            action_id: action.action_id.clone(),
            observed_at: action.start_time,
            evidence: action.evidence.clone(),
        };
        self.llm_agents
            .entry(action.process.clone())
            .or_insert(evidence);

        let mut actions = Vec::new();
        if let Some(mut process_action) = self.process_execs.get(&action.process).cloned() {
            if let Some(evidence) = self.llm_agents.get(&action.process) {
                apply_agent_identity(&mut process_action, evidence);
                self.process_execs
                    .insert(process_action.process.clone(), process_action.clone());
                actions.push(process_action);
            }
        }
        if let Some(invocation) = self.try_agent_invocation(&action.process) {
            actions.push(invocation);
        }
        actions
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.process_execs
            .retain(|_, action| action.trace_id != trace_id);
        self.processes_by_pid
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.direct_edges
            .retain(|_, edge| edge.trace_id != trace_id);
        self.llm_agents
            .retain(|_, evidence| evidence.trace_id != trace_id);
        self.emitted_invocations
            .retain(|key| key.trace_id != trace_id);
    }

    fn merge_process_exec(&self, mut action: SemanticAction) -> SemanticAction {
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
        action
    }

    fn add_candidate_hint(&self, action: &mut SemanticAction) {
        if let Some(command) = matching_agent_command(&self.commands, action) {
            action.attributes.insert(
                ATTR_AGENT_CANDIDATE_COMMAND_MATCH.to_string(),
                "true".to_string(),
            );
            action
                .attributes
                .insert(ATTR_AGENT_CANDIDATE_COMMAND.to_string(), command);
        }
    }

    fn direct_edge(&self, event: &DomainEvent) -> Option<DirectProcessEdge> {
        let EventPayload::Process(payload) = &event.payload else {
            return None;
        };
        let parent_pid = payload
            .metadata
            .get("ppid")
            .or_else(|| payload.metadata.get("stat_ppid"))
            .and_then(|value| value.parse::<u32>().ok())
            .or_else(|| payload.parent.as_ref().map(|parent| parent.pid));
        let parent = payload.parent.clone().or_else(|| {
            parent_pid.and_then(|pid| {
                self.processes_by_pid
                    .get(&(event.envelope.trace_id, pid))
                    .cloned()
            })
        });
        if parent_pid.is_none() && parent.is_none() {
            return None;
        }
        Some(DirectProcessEdge {
            trace_id: event.envelope.trace_id,
            child: event.envelope.process.clone(),
            parent,
            parent_pid,
            observed_at: event.envelope.observed_at,
            exec_event_id: event.envelope.event_id,
        })
    }

    fn try_agent_invocation(&mut self, child: &ProcessIdentity) -> Option<SemanticAction> {
        if !self.invocation_enabled || !self.llm_agents.contains_key(child) {
            return None;
        }
        let edge = self.direct_edges.get(child)?.clone();
        if edge.parent.as_ref().is_some_and(|parent| {
            parent.pid == edge.child.pid && parent.generation == edge.child.generation
        }) {
            return None;
        }
        let key = AgentInvocationKey {
            trace_id: edge.trace_id,
            parent: edge.parent.clone(),
            parent_pid: edge.parent_pid,
            child: edge.child.clone(),
        };
        if !self.emitted_invocations.insert(key) {
            return None;
        }
        let child_action = self.process_execs.get(&edge.child)?;
        let evidence = self.llm_agents.get(&edge.child)?;
        Some(self.agent_invocation_action(&edge, child_action, evidence))
    }

    fn agent_invocation_action(
        &self,
        edge: &DirectProcessEdge,
        child_action: &SemanticAction,
        evidence: &AgentEvidence,
    ) -> SemanticAction {
        let mut attributes = BTreeMap::new();
        attributes.insert(
            ATTR_AGENT_INVOCATION_TRIGGER.to_string(),
            AGENT_INVOCATION_TRIGGER_CHILD_LLM_REQUEST.to_string(),
        );
        attributes.insert(
            ATTR_AGENT_INVOCATION_EVIDENCE_ACTION_ID.to_string(),
            evidence.action_id.clone(),
        );
        attributes.insert("agent.child.pid".to_string(), edge.child.pid.to_string());
        attributes.insert(
            "agent.child.generation".to_string(),
            edge.child.generation.to_string(),
        );
        attributes.insert(
            "agent.child.executable".to_string(),
            executable(child_action),
        );
        if let Some(command_line) = command_line(child_action) {
            attributes.insert("agent.child.command_line".to_string(), command_line);
        }
        if let Some(parent_pid) = edge
            .parent_pid
            .or_else(|| edge.parent.as_ref().map(|parent| parent.pid))
        {
            attributes.insert("agent.parent.pid".to_string(), parent_pid.to_string());
        }
        if let Some(parent) = &edge.parent {
            attributes.insert(
                "agent.parent.generation".to_string(),
                parent.generation.to_string(),
            );
            if let Some(parent_action) = self.process_execs.get(parent) {
                attributes.insert(
                    "agent.parent.executable".to_string(),
                    executable(parent_action),
                );
                if let Some(command_line) = command_line(parent_action) {
                    attributes.insert("agent.parent.command_line".to_string(), command_line);
                }
                let status = if parent_action
                    .attributes
                    .get(ATTR_AGENT_IDENTITY_STATUS)
                    .is_some_and(|status| status == AGENT_IDENTITY_STATUS_OBSERVED)
                {
                    AGENT_PARENT_STATUS_OBSERVED
                } else {
                    AGENT_PARENT_STATUS_UNKNOWN
                };
                attributes.insert(
                    "agent.parent.identity_status".to_string(),
                    status.to_string(),
                );
            }
        }
        let mut action_evidence = Vec::new();
        action_evidence.push(SemanticEvidence {
            kind: semantic_action::SemanticEvidenceKind::Event,
            id: edge.exec_event_id.get(),
            role: "agent.invocation.exec".to_string(),
        });
        action_evidence.extend(evidence.evidence.clone());
        SemanticAction {
            action_id: agent_invocation_action_id(edge.trace_id, &edge.child),
            trace_id: edge.trace_id,
            kind: SemanticActionKind::AgentInvocation,
            title: invocation_title(&attributes, child_action),
            start_time: edge.observed_at,
            end_time: Some(evidence.observed_at),
            process: edge.child.clone(),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence: action_evidence,
        }
    }
}

fn apply_agent_identity(action: &mut SemanticAction, evidence: &AgentEvidence) {
    action.attributes.insert(
        ATTR_AGENT_IDENTITY_STATUS.to_string(),
        AGENT_IDENTITY_STATUS_OBSERVED.to_string(),
    );
    action.attributes.insert(
        ATTR_AGENT_IDENTITY_SOURCE.to_string(),
        AGENT_IDENTITY_SOURCE_LLM_REQUEST.to_string(),
    );
    action.attributes.insert(
        ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID.to_string(),
        evidence.action_id.clone(),
    );
}

fn matching_agent_command(commands: &[String], action: &SemanticAction) -> Option<String> {
    let executable = executable(action);
    let basename = executable.rsplit('/').next().unwrap_or(executable.as_str());
    commands
        .iter()
        .find(|command| command.as_str() == executable || command.as_str() == basename)
        .cloned()
}

fn executable(action: &SemanticAction) -> String {
    action
        .attributes
        .get("process.executable")
        .or_else(|| action.attributes.get("executable"))
        .cloned()
        .unwrap_or_else(|| action.title.clone())
}

fn command_line(action: &SemanticAction) -> Option<String> {
    action.attributes.get("command_line").cloned()
}

fn invocation_title(
    attributes: &BTreeMap<String, String>,
    child_action: &SemanticAction,
) -> String {
    let child = attributes
        .get("agent.child.executable")
        .cloned()
        .unwrap_or_else(|| child_action.title.clone());
    attributes
        .get("agent.parent.executable")
        .map(|parent| format!("{parent} -> {child}"))
        .unwrap_or(child)
}

fn agent_invocation_action_id(trace_id: TraceId, child: &ProcessIdentity) -> String {
    process_action_id(trace_id, child, "agent.invocation")
}

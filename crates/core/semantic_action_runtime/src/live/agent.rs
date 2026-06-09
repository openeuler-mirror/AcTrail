//! Agent identity and direct invocation projection.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence,
};

use super::actions::{
    ACTION_VALID_FALSE, ATTR_ACTION_VALID, ATTR_AGENT_IDENTITY_EVIDENCE_ACTION_ID,
    ATTR_AGENT_IDENTITY_SOURCE, ATTR_AGENT_IDENTITY_STATUS,
    ATTR_AGENT_INVOCATION_EVIDENCE_ACTION_ID, ATTR_AGENT_INVOCATION_TRIGGER,
    append_missing_evidence, event_evidence, process_action_id, process_exit_status,
};
use super::process_parent::{ForkProcessEdge, fork_edge_from_event, merge_fork_edges};

const AGENT_IDENTITY_STATUS_OBSERVED: &str = "observed";
const AGENT_IDENTITY_SOURCE_LLM_REQUEST: &str = "llm.request";
const AGENT_INVOCATION_TRIGGER_CHILD_LLM_REQUEST: &str = "child_llm_request";
const AGENT_PARENT_STATUS_UNKNOWN: &str = "unknown";
const AGENT_PARENT_STATUS_OBSERVED: &str = "observed";
const AGENT_INVOCATION_EXEC_EVIDENCE: &str = "agent.invocation.exec";
const AGENT_INVOCATION_PARENT_FORK_EVIDENCE: &str = "agent.invocation.parent_fork";

pub(super) struct AgentProjector {
    process_execs: BTreeMap<(TraceId, ProcessIdentity), SemanticAction>,
    fork_edges: BTreeMap<(TraceId, ProcessIdentity), ForkProcessEdge>,
    llm_agents: BTreeMap<(TraceId, ProcessIdentity), AgentEvidence>,
    emitted_invocations: BTreeSet<AgentInvocationKey>,
    invocation_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentEvidence {
    action_id: String,
    observed_at: SystemTime,
    evidence: Vec<SemanticEvidence>,
}

type AgentInvocationKey = (TraceId, ProcessIdentity);

impl AgentProjector {
    pub(super) fn new(invocation_enabled: bool) -> Self {
        Self {
            process_execs: BTreeMap::new(),
            fork_edges: BTreeMap::new(),
            llm_agents: BTreeMap::new(),
            emitted_invocations: BTreeSet::new(),
            invocation_enabled,
        }
    }

    pub(super) fn observe_process_exec(
        &mut self,
        _event: &DomainEvent,
        action: SemanticAction,
    ) -> Vec<SemanticAction> {
        let mut action = self.merge_process_exec(action);
        let key = action_key(action.trace_id, &action.process);
        if let Some(evidence) = self.llm_agents.get(&key) {
            apply_agent_identity(&mut action, evidence);
        }
        self.process_execs.insert(key, action.clone());

        let mut actions = vec![action.clone()];
        if let Some(invocation) = self.try_agent_invocation(action.trace_id, &action.process) {
            actions.push(invocation);
        }
        actions
    }

    pub(super) fn observe_process_fork(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        let Some(edge) = fork_edge_from_event(event) else {
            return Vec::new();
        };
        self.merge_fork_edge(edge.clone());
        let key = action_key(edge.trace_id, &edge.child);
        let Some(edge) = self.fork_edges.get(&key).cloned() else {
            return Vec::new();
        };
        if edge.conflict {
            return self
                .invalidated_agent_invocation(&edge)
                .into_iter()
                .collect();
        }
        self.try_agent_invocation(edge.trace_id, &edge.child)
            .into_iter()
            .collect()
    }

    pub(super) fn observe_process_exit(&mut self, event: &DomainEvent) -> Vec<SemanticAction> {
        let EventPayload::Process(payload) = &event.payload else {
            return Vec::new();
        };
        let key = action_key(event.envelope.trace_id, &event.envelope.process);
        self.process_execs
            .get(&key)
            .cloned()
            .map(|mut action| {
                action.end_time = Some(event.envelope.observed_at);
                action.status = process_exit_status(payload.metadata.get("exit_code"));
                action.evidence.push(event_evidence(event, "process.exit"));
                self.process_execs.insert(key, action.clone());
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
            action_id: action.action_id.clone(),
            observed_at: action.start_time,
            evidence: action.evidence.clone(),
        };
        let key = action_key(action.trace_id, &action.process);
        self.llm_agents.entry(key.clone()).or_insert(evidence);

        let mut actions = Vec::new();
        if let Some(mut process_action) = self.process_execs.get(&key).cloned() {
            if let Some(evidence) = self.llm_agents.get(&key) {
                apply_agent_identity(&mut process_action, evidence);
                self.process_execs
                    .insert(key.clone(), process_action.clone());
                actions.push(process_action);
            }
        }
        if let Some(invocation) = self.try_agent_invocation(action.trace_id, &action.process) {
            actions.push(invocation);
        }
        actions
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.process_execs
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.fork_edges
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.llm_agents
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.emitted_invocations
            .retain(|(candidate, _)| *candidate != trace_id);
    }

    fn merge_process_exec(&self, mut action: SemanticAction) -> SemanticAction {
        if let Some(existing) = self
            .process_execs
            .get(&action_key(action.trace_id, &action.process))
        {
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
            if existing.start_time < action.start_time {
                action.start_time = existing.start_time;
            }
            if existing.end_time.is_some() {
                action.end_time = existing.end_time;
                action.status = existing.status;
            }
            append_missing_evidence(&mut action.evidence, &existing.evidence);
        }
        action
    }

    fn merge_fork_edge(&mut self, edge: ForkProcessEdge) {
        let key = action_key(edge.trace_id, &edge.child);
        let merged = merge_fork_edges(self.fork_edges.get(&key), edge);
        self.fork_edges.insert(key, merged);
    }

    fn invalidated_agent_invocation(&self, edge: &ForkProcessEdge) -> Option<SemanticAction> {
        if !self
            .emitted_invocations
            .contains(&(edge.trace_id, edge.child.clone()))
        {
            return None;
        }
        let mut attributes = BTreeMap::from([(
            ATTR_ACTION_VALID.to_string(),
            ACTION_VALID_FALSE.to_string(),
        )]);
        if let Some(evidence) = self.llm_agents.get(&action_key(edge.trace_id, &edge.child)) {
            attributes.insert(
                ATTR_AGENT_INVOCATION_EVIDENCE_ACTION_ID.to_string(),
                evidence.action_id.clone(),
            );
        }
        Some(SemanticAction {
            action_id: agent_invocation_action_id(edge.trace_id, &edge.child),
            trace_id: edge.trace_id,
            kind: SemanticActionKind::AgentInvocation,
            title: "invalid agent invocation".to_string(),
            start_time: edge.observed_at,
            end_time: Some(edge.observed_at),
            process: edge.child.clone(),
            status: SemanticActionStatus::Error,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence: invocation_parent_fork_evidence(edge),
        })
    }

    fn try_agent_invocation(
        &mut self,
        trace_id: TraceId,
        child: &ProcessIdentity,
    ) -> Option<SemanticAction> {
        let child_key = action_key(trace_id, child);
        if !self.invocation_enabled || !self.llm_agents.contains_key(&child_key) {
            return None;
        }
        let edge = self.fork_edges.get(&child_key)?.clone();
        if edge.conflict {
            return None;
        }
        let parent = edge.parent.clone()?;
        if parent == edge.child {
            return None;
        }
        let child_action = self.process_execs.get(&child_key)?;
        let evidence = self.llm_agents.get(&child_key)?;
        if !self
            .emitted_invocations
            .insert((edge.trace_id, edge.child.clone()))
        {
            return None;
        }
        Some(self.agent_invocation_action(&edge, child_action, evidence))
    }

    fn agent_invocation_action(
        &self,
        edge: &ForkProcessEdge,
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
        if let Some(parent) = &edge.parent {
            attributes.insert("agent.parent.pid".to_string(), parent.pid.to_string());
            attributes.insert(
                "agent.parent.generation".to_string(),
                parent.generation.to_string(),
            );
            if let Some(parent_action) = self.process_execs.get(&action_key(edge.trace_id, parent))
            {
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
        let mut action_evidence = invocation_exec_evidence(child_action);
        action_evidence.extend(invocation_parent_fork_evidence(edge));
        action_evidence.extend(evidence.evidence.clone());
        SemanticAction {
            action_id: agent_invocation_action_id(edge.trace_id, &edge.child),
            trace_id: edge.trace_id,
            kind: SemanticActionKind::AgentInvocation,
            title: invocation_title(&attributes, child_action),
            start_time: edge.observed_at.min(evidence.observed_at),
            end_time: Some(edge.observed_at.max(evidence.observed_at)),
            process: edge.child.clone(),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            confidence_millis: None,
            attributes,
            evidence: action_evidence,
        }
    }
}

fn invocation_exec_evidence(child_action: &SemanticAction) -> Vec<SemanticEvidence> {
    child_action
        .evidence
        .iter()
        .filter(|evidence| evidence.role == "process.exec")
        .map(|evidence| SemanticEvidence {
            kind: evidence.kind,
            id: evidence.id,
            role: AGENT_INVOCATION_EXEC_EVIDENCE.to_string(),
        })
        .collect()
}

fn invocation_parent_fork_evidence(edge: &ForkProcessEdge) -> Vec<SemanticEvidence> {
    edge.evidence
        .iter()
        .map(|evidence| SemanticEvidence {
            kind: evidence.kind,
            id: evidence.id,
            role: AGENT_INVOCATION_PARENT_FORK_EVIDENCE.to_string(),
        })
        .collect()
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

fn action_key(trace_id: TraceId, process: &ProcessIdentity) -> (TraceId, ProcessIdentity) {
    (trace_id, process.clone())
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

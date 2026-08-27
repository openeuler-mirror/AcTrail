//! One-shot command invocation projection from completed process exec actions.

use std::collections::BTreeMap;

use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticEvidence, attr_keys as attrs,
    evidence_roles,
};

use super::actions::{
    ATTR_PROCESS_PARENT_IDENTITY_STATE, append_missing_evidence, event_action_id,
};
use super::process_parent::{
    ForkProcessEdge, apply_fork_parent, fork_edge_from_event, is_parent_identity_attr,
    merge_fork_edges,
};
use super::runtime::LiveSemanticActionOutput;

type CommandKey = (TraceId, ProcessIdentity);

pub(super) struct CommandProjector {
    commands: BTreeMap<CommandKey, SemanticAction>,
    fork_edges: BTreeMap<CommandKey, ForkProcessEdge>,
    mcp_invocations: BTreeMap<CommandKey, McpInvocationEvidence>,
}

#[derive(Clone)]
struct McpInvocationEvidence {
    evidence: Vec<SemanticEvidence>,
}

impl CommandProjector {
    pub(super) fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
            fork_edges: BTreeMap::new(),
            mcp_invocations: BTreeMap::new(),
        }
    }

    pub(super) fn observe_process_exec(
        &mut self,
        event: &DomainEvent,
        process_action: &SemanticAction,
    ) -> LiveSemanticActionOutput {
        let mut action = command_action(event, process_action);
        if let Some(edge) = self
            .fork_edges
            .get(&command_key(action.trace_id, &action.process))
        {
            apply_fork_parent(&mut action, edge);
        }
        let key = command_key(action.trace_id, &action.process);
        if let Some(evidence) = self.mcp_invocations.get(&key) {
            Self::apply_mcp_invocation(&mut action, evidence);
        }
        self.commands.insert(key, action.clone());
        let link = command_exec_link(&action, process_action);
        LiveSemanticActionOutput {
            actions: vec![action],
            links: vec![link],
            ..LiveSemanticActionOutput::default()
        }
    }

    pub(super) fn observe_mcp_tool_call(
        &mut self,
        action: &SemanticAction,
    ) -> LiveSemanticActionOutput {
        if action.kind != SemanticActionKind::McpToolCall
            || action
                .attributes
                .get(attrs::mcp::TRANSPORT)
                .map(String::as_str)
                != Some("stdio")
        {
            return LiveSemanticActionOutput::default();
        }
        let key = command_key(action.trace_id, &action.process);
        let invocation =
            self.mcp_invocations
                .entry(key.clone())
                .or_insert_with(|| McpInvocationEvidence {
                    evidence: Vec::new(),
                });
        append_missing_evidence(&mut invocation.evidence, &action.evidence);
        let invocation = invocation.clone();
        let Some(mut command) = self.commands.get(&key).cloned() else {
            return LiveSemanticActionOutput::default();
        };
        let previous = command.clone();
        Self::apply_mcp_invocation(&mut command, &invocation);
        if command == previous {
            return LiveSemanticActionOutput::default();
        }
        self.commands.insert(key, command.clone());
        LiveSemanticActionOutput {
            actions: vec![command],
            ..LiveSemanticActionOutput::default()
        }
    }

    pub(super) fn observe_process_fork(&mut self, event: &DomainEvent) {
        let Some(edge) = fork_edge_from_event(event) else {
            return;
        };
        let key = command_key(edge.trace_id, &edge.child);
        let edge = merge_fork_edges(self.fork_edges.get(&key), edge);
        self.fork_edges.insert(key, edge);
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.commands
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.fork_edges
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.mcp_invocations
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    fn apply_mcp_invocation(action: &mut SemanticAction, invocation: &McpInvocationEvidence) {
        if action
            .attributes
            .get(attrs::invocation::KIND)
            .map(String::as_str)
            != Some("agent")
        {
            action
                .attributes
                .insert(attrs::invocation::KIND.to_string(), "mcp".to_string());
        }
        append_missing_evidence(&mut action.evidence, &invocation.evidence);
    }
}

fn command_key(trace_id: TraceId, process: &ProcessIdentity) -> CommandKey {
    (trace_id, process.clone())
}

fn command_action(event: &DomainEvent, process_action: &SemanticAction) -> SemanticAction {
    let mut attributes = BTreeMap::new();
    if let Some(executable) = process_action
        .attributes
        .get(attrs::process::EXECUTABLE)
        .or_else(|| process_action.attributes.get("executable"))
    {
        attributes.insert(attrs::process::EXECUTABLE.to_string(), executable.clone());
    }
    if let Some(command_line) = process_action.attributes.get("command_line") {
        attributes.insert(attrs::command::LINE.to_string(), command_line.clone());
    }
    for key in ["ppid", "stat_ppid"] {
        if let Some(value) = process_action.attributes.get(key) {
            attributes.insert(key.to_string(), value.clone());
        }
    }
    for (key, value) in &process_action.attributes {
        if is_parent_identity_attr(key) || key == ATTR_PROCESS_PARENT_IDENTITY_STATE {
            attributes.insert(key.clone(), value.clone());
        }
    }
    let evidence = process_action
        .evidence
        .iter()
        .cloned()
        .map(|mut evidence| {
            evidence.role = evidence_roles::command::EXEC.to_string();
            evidence
        })
        .collect();
    SemanticAction {
        action_id: event_action_id(event, SemanticActionKind::CommandInvocation.as_str()),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::CommandInvocation,
        title: attributes
            .get(attrs::command::LINE)
            .or_else(|| attributes.get(attrs::process::EXECUTABLE))
            .cloned()
            .unwrap_or_else(|| process_action.title.clone()),
        start_time: process_action.start_time,
        end_time: process_action.end_time,
        process: event.envelope.process.clone(),
        status: process_action.status,
        completeness: SemanticActionCompleteness::Complete,
        attributes,
        evidence,
    }
}

fn command_exec_link(
    action: &SemanticAction,
    process_action: &SemanticAction,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: action.trace_id,
        parent_action_id: action.action_id.clone(),
        child_action_id: process_action.action_id.clone(),
        role: SemanticActionLinkRole::CommandContainsProcessExec,
        confidence: SemanticActionLinkConfidence::Observed,
        valid: true,
        evidence: action.evidence.clone(),
        attributes: BTreeMap::new(),
    }
}

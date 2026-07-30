//! One-shot command invocation projection from completed process exec actions.

use std::collections::BTreeMap;

use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, attr_keys as attrs, evidence_roles,
};

use super::actions::{ATTR_PROCESS_PARENT_IDENTITY_STATE, event_action_id};
use super::process_parent::{
    ForkProcessEdge, apply_fork_parent, fork_edge_from_event, is_parent_identity_attr,
    merge_fork_edges,
};
use super::runtime::LiveSemanticActionOutput;

type CommandKey = (TraceId, ProcessIdentity);

pub(super) struct CommandProjector {
    fork_edges: BTreeMap<CommandKey, ForkProcessEdge>,
}

impl CommandProjector {
    pub(super) fn new() -> Self {
        Self {
            fork_edges: BTreeMap::new(),
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
        let link = command_exec_link(&action, process_action);
        LiveSemanticActionOutput {
            actions: vec![action],
            links: vec![link],
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
        self.fork_edges
            .retain(|(candidate, _), _| *candidate != trace_id);
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
        confidence_millis: None,
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

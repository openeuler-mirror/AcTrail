//! Command invocation projection from process exec/exit actions.

use std::collections::{BTreeMap, BTreeSet};

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole,
};

use super::actions::{
    ATTR_PROCESS_PARENT_IDENTITY_STATE, append_missing_evidence, event_evidence, process_action_id,
    process_exit_status,
};
use super::process_parent::{
    ForkProcessEdge, ParentEdgeApply, apply_fork_parent, fork_edge_from_event,
    is_parent_identity_attr, merge_fork_edges, parent_identity_has_conflict,
};
use super::runtime::LiveSemanticActionOutput;

pub(super) struct CommandProjector {
    commands: BTreeMap<(TraceId, ProcessIdentity), SemanticAction>,
    fork_edges: BTreeMap<(TraceId, ProcessIdentity), ForkProcessEdge>,
    linked_execs: BTreeSet<CommandExecLinkKey>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CommandExecLinkKey {
    trace_id: TraceId,
    parent_action_id: String,
    child_action_id: String,
}

impl CommandProjector {
    pub(super) fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
            fork_edges: BTreeMap::new(),
            linked_execs: BTreeSet::new(),
        }
    }

    pub(super) fn observe_process_exec(
        &mut self,
        event: &DomainEvent,
        process_action: &SemanticAction,
    ) -> LiveSemanticActionOutput {
        let mut action = command_action(event, process_action);
        let key = command_key(action.trace_id, &action.process);
        if let Some(edge) = self.fork_edges.get(&key) {
            apply_fork_parent(&mut action, edge);
        }
        if let Some(existing) = self.commands.get(&key) {
            merge_existing_command(&mut action, existing);
        }
        self.commands.insert(key, action.clone());
        let link = self.command_exec_link(&action, process_action, event);
        LiveSemanticActionOutput {
            actions: vec![action],
            links: link.into_iter().collect(),
        }
    }

    pub(super) fn observe_process_fork(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let Some(edge) = fork_edge_from_event(event) else {
            return LiveSemanticActionOutput::default();
        };
        let key = command_key(edge.trace_id, &edge.child);
        let edge = merge_fork_edges(self.fork_edges.get(&key), edge);
        self.fork_edges.insert(key.clone(), edge.clone());
        let Some(mut action) = self.commands.get(&key).cloned() else {
            return LiveSemanticActionOutput::default();
        };
        if apply_fork_parent(&mut action, &edge) == ParentEdgeApply::Unchanged {
            return LiveSemanticActionOutput::default();
        }
        self.commands.insert(key, action.clone());
        LiveSemanticActionOutput {
            actions: vec![action],
            links: Vec::new(),
        }
    }

    pub(super) fn observe_process_exit(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let EventPayload::Process(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        let key = command_key(event.envelope.trace_id, &event.envelope.process);
        let Some(mut action) = self.commands.get(&key).cloned() else {
            return LiveSemanticActionOutput::default();
        };
        action.end_time = Some(event.envelope.observed_at);
        action.status = process_exit_status(payload.metadata.get("exit_code"));
        action.evidence.push(event_evidence(event, "process.exit"));
        self.commands.insert(key, action.clone());
        LiveSemanticActionOutput {
            actions: vec![action],
            links: Vec::new(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.commands
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.fork_edges
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.linked_execs.retain(|key| key.trace_id != trace_id);
    }

    fn command_exec_link(
        &mut self,
        action: &SemanticAction,
        process_action: &SemanticAction,
        event: &DomainEvent,
    ) -> Option<SemanticActionLink> {
        let key = CommandExecLinkKey {
            trace_id: action.trace_id,
            parent_action_id: action.action_id.clone(),
            child_action_id: process_action.action_id.clone(),
        };
        if !self.linked_execs.insert(key) {
            return None;
        }
        Some(SemanticActionLink {
            trace_id: action.trace_id,
            parent_action_id: action.action_id.clone(),
            child_action_id: process_action.action_id.clone(),
            role: SemanticActionLinkRole::CommandContainsProcessExec,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: vec![event_evidence(event, "command.exec")],
            attributes: BTreeMap::new(),
        })
    }
}

fn command_key(trace_id: TraceId, process: &ProcessIdentity) -> (TraceId, ProcessIdentity) {
    (trace_id, process.clone())
}

fn command_action(event: &DomainEvent, process_action: &SemanticAction) -> SemanticAction {
    let mut attributes = BTreeMap::new();
    if let Some(executable) = process_action
        .attributes
        .get("process.executable")
        .or_else(|| process_action.attributes.get("executable"))
    {
        attributes.insert("process.executable".to_string(), executable.clone());
    }
    if let Some(command_line) = process_action.attributes.get("command_line") {
        attributes.insert("command.line".to_string(), command_line.clone());
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
    SemanticAction {
        action_id: process_action_id(
            event.envelope.trace_id,
            &event.envelope.process,
            "command.invocation",
        ),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::CommandInvocation,
        title: attributes
            .get("command.line")
            .or_else(|| attributes.get("process.executable"))
            .cloned()
            .unwrap_or_else(|| process_action.title.clone()),
        start_time: event.envelope.observed_at,
        end_time: process_action.end_time,
        process: event.envelope.process.clone(),
        status: process_action.status,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, "process.exec")],
    }
}

fn merge_existing_command(action: &mut SemanticAction, existing: &SemanticAction) {
    let parent_conflict =
        parent_identity_has_conflict(existing) || parent_identity_has_conflict(action);
    for (key, value) in &existing.attributes {
        if parent_conflict && is_parent_identity_attr(key) {
            continue;
        }
        action
            .attributes
            .entry(key.clone())
            .or_insert_with(|| value.clone());
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

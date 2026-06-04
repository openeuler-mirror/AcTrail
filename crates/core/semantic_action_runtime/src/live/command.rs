//! Command invocation projection from process exec/exit actions.

use std::collections::{BTreeMap, BTreeSet};

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
};

use super::actions::{event_evidence, process_action_id, process_exit_status};
use super::runtime::LiveSemanticActionOutput;

pub(super) struct CommandProjector {
    commands: BTreeMap<ProcessIdentity, SemanticAction>,
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
            linked_execs: BTreeSet::new(),
        }
    }

    pub(super) fn observe_process_exec(
        &mut self,
        event: &DomainEvent,
        process_action: &SemanticAction,
    ) -> LiveSemanticActionOutput {
        let action = command_action(event, process_action);
        self.commands.insert(action.process.clone(), action.clone());
        let link = self.command_exec_link(&action, process_action, event);
        LiveSemanticActionOutput {
            actions: vec![action],
            links: link.into_iter().collect(),
        }
    }

    pub(super) fn observe_process_exit(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let EventPayload::Process(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        let Some(mut action) = self.commands.get(&event.envelope.process).cloned() else {
            return LiveSemanticActionOutput::default();
        };
        action.end_time = Some(event.envelope.observed_at);
        action.status = process_exit_status(payload.metadata.get("exit_code"));
        action.evidence.push(event_evidence(event, "process.exit"));
        self.commands
            .insert(event.envelope.process.clone(), action.clone());
        LiveSemanticActionOutput {
            actions: vec![action],
            links: Vec::new(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.commands
            .retain(|_, action| action.trace_id != trace_id);
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
        end_time: None,
        process: event.envelope.process.clone(),
        status: SemanticActionStatus::InProgress,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, "process.exec")],
    }
}

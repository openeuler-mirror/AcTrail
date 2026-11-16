//! Command context projection from successful process exec actions.

use std::collections::{BTreeMap, btree_map::Entry};
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionChange, SemanticActionCompleteness,
    SemanticActionFinalizationReason, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkOrigin, SemanticActionLinkRole, SemanticActionStatus, SemanticActionUpdate,
    SemanticCommandKind, SemanticEvidence, attr_keys as attrs, evidence_roles,
};

use super::actions::{
    ATTR_PROCESS_PARENT_IDENTITY_STATE, append_missing_evidence, event_action_id, event_evidence,
    process_exit_status,
};
use super::process_parent::{
    ForkProcessEdge, apply_fork_parent, fork_edge_from_event, is_parent_identity_attr,
    merge_fork_edges,
};
use super::runtime::LiveSemanticActionOutput;

type CommandKey = (TraceId, ProcessIdentity);

pub(super) struct CommandProjector {
    commands: BTreeMap<CommandKey, PendingCommand>,
    fork_edges: BTreeMap<CommandKey, ForkProcessEdge>,
    mcp_invocations: BTreeMap<CommandKey, McpInvocationEvidence>,
}

struct McpInvocationEvidence {
    evidence: Vec<SemanticEvidence>,
}

struct PendingCommand {
    action_id: String,
    start_time: SystemTime,
    end_time: Option<SystemTime>,
    status: SemanticActionStatus,
    completeness: SemanticActionCompleteness,
    classification: SemanticCommandKind,
    evidence: Vec<SemanticEvidence>,
}

impl PendingCommand {
    fn from_action(action: &SemanticAction) -> Self {
        let classification = match action
            .attributes
            .get(attrs::invocation::KIND)
            .map(String::as_str)
        {
            Some("agent") => SemanticCommandKind::Agent,
            Some("mcp") => SemanticCommandKind::Mcp,
            _ => SemanticCommandKind::Command,
        };
        Self {
            action_id: action.action_id.clone(),
            start_time: action.start_time,
            end_time: action.end_time,
            status: action.status,
            completeness: action.completeness,
            classification,
            evidence: action.evidence.clone(),
        }
    }

    fn lifecycle(
        &self,
        key: CommandKey,
        command_exit_code: Option<i32>,
        finalization_reason: Option<SemanticActionFinalizationReason>,
        evidence: Vec<SemanticEvidence>,
    ) -> SemanticActionUpdate {
        self.update(
            key,
            SemanticActionChange::Lifecycle {
                end_time: self.end_time,
                status: self.status,
                completeness: self.completeness,
                finalization_reason,
                command_exit_code,
            },
            evidence,
        )
    }

    fn update(
        &self,
        key: CommandKey,
        change: SemanticActionChange,
        evidence: Vec<SemanticEvidence>,
    ) -> SemanticActionUpdate {
        SemanticActionUpdate {
            action_id: self.action_id.clone(),
            trace_id: key.0,
            kind: SemanticActionKind::CommandInvocation,
            process: key.1,
            change,
            evidence,
        }
    }

    fn mark_end_unknown(&mut self) {
        self.end_time = None;
        self.status = SemanticActionStatus::Unknown;
        self.completeness = SemanticActionCompleteness::CaptureLimited;
    }

    fn is_running(&self) -> bool {
        self.end_time.is_none() && self.completeness == SemanticActionCompleteness::Partial
    }

    fn apply_mcp_invocation(&mut self, invocation: &McpInvocationEvidence) -> bool {
        let previous_kind = self.classification;
        let previous_evidence_count = self.evidence.len();
        if self.classification != SemanticCommandKind::Agent {
            self.classification = SemanticCommandKind::Mcp;
        }
        append_missing_evidence(&mut self.evidence, &invocation.evidence);
        self.classification != previous_kind || self.evidence.len() != previous_evidence_count
    }
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
        if process_action.status != SemanticActionStatus::Success {
            return LiveSemanticActionOutput::default();
        }
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
        let mut output = LiveSemanticActionOutput::default();
        if let Some(previous) = self.commands.get_mut(&key) {
            if previous.is_running() {
                if event.envelope.observed_at >= previous.start_time {
                    previous.end_time = Some(event.envelope.observed_at);
                    previous.status = SemanticActionStatus::Unknown;
                    previous.completeness = SemanticActionCompleteness::Complete;
                } else {
                    previous.mark_end_unknown();
                }
                append_missing_evidence(
                    &mut previous.evidence,
                    &[event_evidence(
                        event,
                        evidence_roles::process::EXEC_COMPLETED,
                    )],
                );
                output
                    .updates
                    .push(previous.lifecycle(key, None, None, previous.evidence.clone()));
            }
        }
        self.commands
            .insert(key, PendingCommand::from_action(&action));
        let link = command_exec_link(&action, process_action);
        output.actions.push(action);
        output.links.push(link);
        output
    }

    pub(super) fn observe_process_exit(
        &mut self,
        event: &DomainEvent,
        retain_for_mcp: bool,
    ) -> LiveSemanticActionOutput {
        let EventPayload::Process(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        if payload.operation != "exit" {
            return LiveSemanticActionOutput::default();
        }
        let key = command_key(event.envelope.trace_id, &event.envelope.process);
        let Entry::Occupied(mut entry) = self.commands.entry(key) else {
            return LiveSemanticActionOutput::default();
        };
        let action = entry.get_mut();
        if action.end_time.is_some() {
            return LiveSemanticActionOutput::default();
        }
        let observed_exit = event.envelope.collector.as_str() == "ebpf"
            && payload.metadata.get("exit_source").map(String::as_str) != Some("reconciled")
            && event.envelope.observed_at >= action.start_time;
        let exit_code = observed_exit
            .then(|| {
                payload
                    .metadata
                    .get("exit_code")
                    .and_then(|raw| raw.parse::<i32>().ok())
            })
            .flatten();
        if observed_exit {
            action.end_time = Some(event.envelope.observed_at);
            action.status = process_exit_status(exit_code);
            action.completeness = SemanticActionCompleteness::Complete;
        } else {
            action.mark_end_unknown();
        }
        let exit_evidence = event_evidence(event, evidence_roles::process::EXIT);
        append_missing_evidence(&mut action.evidence, std::slice::from_ref(&exit_evidence));
        let mut output = LiveSemanticActionOutput::default();
        output
            .updates
            .push(action.lifecycle(key, exit_code, None, vec![exit_evidence]));
        if !retain_for_mcp {
            entry.remove();
        }
        output
    }

    pub(super) fn finalize_trace(&mut self, trace_id: TraceId) -> LiveSemanticActionOutput {
        let mut output = LiveSemanticActionOutput::default();
        for (key, action) in &mut self.commands {
            if key.0 != trace_id || !action.is_running() {
                continue;
            }
            action.mark_end_unknown();
            output.updates.push(action.lifecycle(
                *key,
                None,
                Some(SemanticActionFinalizationReason::TraceClosed),
                action.evidence.clone(),
            ));
        }
        self.forget_trace(trace_id);
        output
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
        let invocation = self
            .mcp_invocations
            .entry(key)
            .or_insert_with(|| McpInvocationEvidence {
                evidence: Vec::new(),
            });
        append_missing_evidence(&mut invocation.evidence, &action.evidence);
        let Some(command) = self.commands.get_mut(&key) else {
            return LiveSemanticActionOutput::default();
        };
        if !command.apply_mcp_invocation(invocation) {
            return LiveSemanticActionOutput::default();
        }
        let update = command.update(
            key,
            SemanticActionChange::CommandClassification {
                kind: command.classification,
            },
            command.evidence.clone(),
        );
        LiveSemanticActionOutput {
            updates: vec![update],
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
        end_time: None,
        process: event.envelope.process.clone(),
        // A successful exec has been observed; its execution end is still pending.
        status: process_action.status,
        completeness: SemanticActionCompleteness::Partial,
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
        origin: SemanticActionLinkOrigin::Observed,
        valid: true,
        evidence: action.evidence.clone(),
        attributes: BTreeMap::new(),
    }
}

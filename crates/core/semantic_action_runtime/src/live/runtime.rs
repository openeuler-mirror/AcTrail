//! Live semantic action runtime.

use config_core::daemon::AgentInvocationConfig;
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use semantic_action::{
    SemanticAction, SemanticActionLink, SemanticActionLinkConfidence, SemanticActionLinkRole,
};

use super::actions::{
    enforcement_action, file_modify_action, http_message_action, is_file_modify_operation,
    is_http_protocol, process_action_id, process_exec_action, process_fork_attempt_action,
};
use super::agent::AgentProjector;
use super::command::CommandProjector;
use super::file::FileWriteProjector;
use super::llm::LiveLlmProjector;

pub struct LiveSemanticActionRuntime {
    agent: AgentProjector,
    command: CommandProjector,
    file_write: FileWriteProjector,
    llm: LiveLlmProjector,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LiveSemanticActionOutput {
    pub actions: Vec<SemanticAction>,
    pub links: Vec<SemanticActionLink>,
}

impl LiveSemanticActionOutput {
    fn from_actions(actions: Vec<SemanticAction>) -> Self {
        Self {
            actions,
            links: Vec::new(),
        }
    }

    fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
    }
}

impl LiveSemanticActionRuntime {
    pub fn new(config: AgentInvocationConfig) -> Self {
        Self {
            agent: AgentProjector::new(config.enabled, config.commands),
            command: CommandProjector::new(),
            file_write: FileWriteProjector::new(),
            llm: LiveLlmProjector::default(),
        }
    }

    pub fn observe_event(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        match &event.payload {
            EventPayload::Process(payload) if payload.operation == "exec" => {
                let actions = self
                    .agent
                    .observe_process_exec(event, process_exec_action(event));
                let mut output = LiveSemanticActionOutput::from_actions(actions.clone());
                if let Some(process_action) = actions
                    .iter()
                    .find(|action| action.kind == semantic_action::SemanticActionKind::ProcessExec)
                {
                    output.extend(self.command.observe_process_exec(event, process_action));
                }
                output.extend(agent_invocation_links(&actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork_attempt" => {
                LiveSemanticActionOutput::from_actions(vec![process_fork_attempt_action(event)])
            }
            EventPayload::Process(payload) if payload.operation == "exit" => {
                let mut output =
                    LiveSemanticActionOutput::from_actions(self.agent.observe_process_exit(event));
                output.extend(self.command.observe_process_exit(event));
                output
            }
            EventPayload::File(payload) if is_file_modify_operation(&payload.operation) => {
                let file_action = file_modify_action(event);
                let mut output = LiveSemanticActionOutput::from_actions(vec![file_action.clone()]);
                output.extend(
                    self.file_write
                        .observe_file_event(event, Some(&file_action)),
                );
                output
            }
            EventPayload::File(_) => self.file_write.observe_file_event(event, None),
            EventPayload::Application(payload) if is_http_protocol(&payload.protocol) => {
                LiveSemanticActionOutput::from_actions(vec![http_message_action(event)])
            }
            EventPayload::Enforcement(_) => {
                LiveSemanticActionOutput::from_actions(vec![enforcement_action(event)])
            }
            _ => LiveSemanticActionOutput::default(),
        }
    }

    pub fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        let llm_actions = self.llm.observe_payload_segment(segment);
        let mut output = LiveSemanticActionOutput::default();
        for action in llm_actions {
            let agent_actions = self.agent.observe_llm_request(&action);
            output
                .links
                .extend(agent_performed_action_links(&agent_actions, &action));
            output.actions.push(action.clone());
            output.actions.extend(agent_actions.clone());
            output.extend(agent_invocation_links(&agent_actions));
        }
        output
    }

    pub fn forget_trace(&mut self, trace_id: TraceId) {
        self.agent.forget_trace(trace_id);
        self.command.forget_trace(trace_id);
        self.file_write.forget_trace(trace_id);
        self.llm.forget_trace(trace_id);
    }
}

fn agent_performed_action_links(
    agent_actions: &[SemanticAction],
    child_action: &SemanticAction,
) -> Vec<SemanticActionLink> {
    agent_actions
        .iter()
        .filter(|action| action.kind == semantic_action::SemanticActionKind::ProcessExec)
        .filter(|action| {
            action
                .attributes
                .get("agent.identity.status")
                .is_some_and(|status| status == "observed")
        })
        .map(|agent_action| SemanticActionLink {
            trace_id: child_action.trace_id,
            parent_action_id: agent_action.action_id.clone(),
            child_action_id: child_action.action_id.clone(),
            role: SemanticActionLinkRole::AgentPerformedAction,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: child_action.evidence.clone(),
            attributes: std::collections::BTreeMap::new(),
        })
        .collect()
}

fn agent_invocation_links(actions: &[SemanticAction]) -> LiveSemanticActionOutput {
    let mut links = Vec::new();
    for action in actions
        .iter()
        .filter(|action| action.kind == semantic_action::SemanticActionKind::AgentInvocation)
    {
        links.push(SemanticActionLink {
            trace_id: action.trace_id,
            parent_action_id: action.action_id.clone(),
            child_action_id: process_action_id(action.trace_id, &action.process, "exec"),
            role: SemanticActionLinkRole::AgentInvocationExec,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: action
                .evidence
                .iter()
                .filter(|evidence| evidence.role == "agent.invocation.exec")
                .cloned()
                .collect(),
            attributes: std::collections::BTreeMap::new(),
        });
        if let Some(child_llm_action_id) = action
            .attributes
            .get("agent.invocation.evidence_action_id")
            .cloned()
        {
            links.push(SemanticActionLink {
                trace_id: action.trace_id,
                parent_action_id: action.action_id.clone(),
                child_action_id: child_llm_action_id,
                role: SemanticActionLinkRole::AgentInvocationChildLlmRequest,
                confidence: SemanticActionLinkConfidence::Observed,
                evidence: action
                    .evidence
                    .iter()
                    .filter(|evidence| evidence.role != "agent.invocation.exec")
                    .cloned()
                    .collect(),
                attributes: std::collections::BTreeMap::new(),
            });
        }
    }
    LiveSemanticActionOutput {
        actions: Vec::new(),
        links,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

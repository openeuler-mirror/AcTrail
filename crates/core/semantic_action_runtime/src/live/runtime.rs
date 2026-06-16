//! Live semantic action runtime.

use config_core::daemon::{AgentInvocationConfig, SemanticRetentionConfig};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLink};
use std::time::SystemTime;

use super::actions::{
    enforcement_action, file_modify_action, http_message_action, is_file_modify_operation,
    is_http_protocol, process_exec_action, process_fork_attempt_action,
};
use super::agent::AgentProjector;
use super::command::CommandProjector;
use super::file::FileAccessProjector;
use super::links::ActionLinkProjector;
use super::llm::LiveLlmProjector;

pub struct LiveSemanticActionRuntime {
    agent: AgentProjector,
    command: CommandProjector,
    file_access: FileAccessProjector,
    llm: LiveLlmProjector,
    links: ActionLinkProjector,
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
    pub fn new(config: AgentInvocationConfig, semantic_retention: SemanticRetentionConfig) -> Self {
        let AgentInvocationConfig {
            enabled,
            commands: _,
        } = config;
        Self {
            agent: AgentProjector::new(enabled),
            command: CommandProjector::new(),
            file_access: FileAccessProjector::new(),
            llm: LiveLlmProjector::new(semantic_retention),
            links: ActionLinkProjector::new(),
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
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork_attempt" => {
                let mut output =
                    LiveSemanticActionOutput::from_actions(vec![process_fork_attempt_action(
                        event,
                    )]);
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "fork" => {
                let mut output = self.command.observe_process_fork(event);
                output.links.extend(self.links.observe_process_fork(event));
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Process(payload) if payload.operation == "exit" => {
                let mut output =
                    LiveSemanticActionOutput::from_actions(self.agent.observe_process_exit(event));
                output.extend(self.command.observe_process_exit(event));
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::File(payload) if is_file_modify_operation(&payload.operation) => {
                let file_action = file_modify_action(event);
                let mut output = LiveSemanticActionOutput::from_actions(vec![file_action.clone()]);
                output.extend(
                    self.file_access
                        .observe_file_event(event, Some(&file_action)),
                );
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::File(_) => {
                let mut output = self.file_access.observe_file_event(event, None);
                output
                    .links
                    .extend(self.links.observe_actions(&output.actions));
                output
            }
            EventPayload::Application(payload) if is_http_protocol(&payload.protocol) => {
                let action = http_message_action(event);
                let mut actions = vec![action.clone()];
                actions.extend(self.llm.observe_http_message(&action));
                let links = self.links.observe_actions(&actions);
                LiveSemanticActionOutput { actions, links }
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
            let agent_actions = if action.kind == SemanticActionKind::LlmRequest {
                self.agent.observe_llm_request(&action)
            } else {
                Vec::new()
            };
            output.actions.push(action.clone());
            output.actions.extend(agent_actions.clone());
            for process_action in agent_actions {
                output.extend(
                    self.command
                        .observe_agent_identity(&process_action, &action),
                );
            }
        }
        output
            .links
            .extend(self.links.observe_actions(&output.actions));
        output
    }

    pub fn forget_trace(&mut self, trace_id: TraceId) {
        self.agent.forget_trace(trace_id);
        self.command.forget_trace(trace_id);
        self.file_access.forget_trace(trace_id);
        self.llm.forget_trace(trace_id);
        self.links.forget_trace(trace_id);
    }

    pub fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let actions = self.llm.finalize_trace(trace_id, finished_at);
        let links = self.links.observe_actions(&actions);
        LiveSemanticActionOutput { actions, links }
    }
}

#[cfg(test)]
#[path = "runtime_tests/support.rs"]
mod test_support;

#[cfg(test)]
#[path = "runtime_tests/process.rs"]
mod process_tests;

#[cfg(test)]
#[path = "runtime_tests/command.rs"]
mod command_tests;

#[cfg(test)]
#[path = "runtime_tests/command_identity.rs"]
mod command_identity_tests;

#[cfg(test)]
#[path = "runtime_tests/llm.rs"]
mod llm_tests;

#[cfg(test)]
#[path = "runtime_tests/llm_links.rs"]
mod llm_link_tests;

#[cfg(test)]
#[path = "runtime_tests/llm_non_llm.rs"]
mod llm_non_llm_tests;

#[cfg(test)]
#[path = "runtime_tests/file.rs"]
mod file_tests;

use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionLink};

use super::agent::AgentPerformedActionLinkProjector;
use super::command::CommandChildActionLinkProjector;
use super::http::HttpMessageLinkProjector;
use super::llm::LlmExchangeLinkProjector;
use super::sse::SseLinkProjector;

pub(in crate::live) struct ActionLinkProjector {
    agent: AgentPerformedActionLinkProjector,
    command: CommandChildActionLinkProjector,
    http: HttpMessageLinkProjector,
    llm_exchange: LlmExchangeLinkProjector,
    sse: SseLinkProjector,
}

impl ActionLinkProjector {
    pub(in crate::live) fn new() -> Self {
        Self {
            agent: AgentPerformedActionLinkProjector::default(),
            command: CommandChildActionLinkProjector::default(),
            http: HttpMessageLinkProjector::default(),
            llm_exchange: LlmExchangeLinkProjector::default(),
            sse: SseLinkProjector::default(),
        }
    }

    pub(in crate::live) fn observe_actions(
        &mut self,
        actions: &[SemanticAction],
    ) -> Vec<SemanticActionLink> {
        for action in actions {
            self.agent.observe_action(action);
            self.command.observe_action(action);
        }

        let mut links = Vec::new();
        for action in actions {
            links.extend(self.agent.link_pending_for_agent(action));
            links.extend(self.command.link_pending_for_command(action));
        }
        for action in actions {
            links.extend(self.http.observe_action(action));
            links.extend(self.llm_exchange.observe_action(action));
            links.extend(self.sse.observe_action(action));
            links.extend(self.agent.link_child_action(action));
            links.extend(self.command.link_child_action(action));
        }
        links
    }

    pub(in crate::live) fn observe_process_fork(
        &mut self,
        event: &DomainEvent,
    ) -> Vec<SemanticActionLink> {
        self.command.observe_process_fork(event)
    }

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.agent.forget_trace(trace_id);
        self.command.forget_trace(trace_id);
        self.http.forget_trace(trace_id);
        self.llm_exchange.forget_trace(trace_id);
        self.sse.forget_trace(trace_id);
    }
}

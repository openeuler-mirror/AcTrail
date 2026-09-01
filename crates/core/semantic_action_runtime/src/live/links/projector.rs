use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionLink};

use super::agent::AgentPerformedActionLinkProjector;
use super::command::CommandChildActionLinkProjector;
use super::http::HttpMessageLinkProjector;
use super::llm::LlmExchangeLinkProjector;
use super::sse::SseLinkProjector;
use crate::live::HttpResponseMatch;
use crate::live::actions::action_for_live_state;
use crate::llm_pipeline::{LlmHttpRequestLink, LlmHttpResponseLink};

pub(in crate::live) struct ActionLinkProjector {
    agent: AgentPerformedActionLinkProjector,
    command: CommandChildActionLinkProjector,
    http: HttpMessageLinkProjector,
    llm_exchange: LlmExchangeLinkProjector,
    sse: SseLinkProjector,
}

impl ActionLinkProjector {
    pub(in crate::live) fn observe_exact_http_exchange_link(
        &self,
        exchange: &HttpResponseMatch,
    ) -> SemanticActionLink {
        self.http.observe_exact_exchange_link(exchange)
    }

    pub(in crate::live) fn observe_exact_http_request_link(
        &mut self,
        proposal: &LlmHttpRequestLink,
    ) -> Option<SemanticActionLink> {
        self.http
            .observe_exact_request_link(&proposal.llm_request, &proposal.http_request)
    }

    pub(in crate::live) fn observe_exact_http_response_link(
        &mut self,
        proposal: &LlmHttpResponseLink,
    ) -> Option<SemanticActionLink> {
        self.http
            .observe_exact_response_link(&proposal.llm_response, &proposal.http_response)
    }

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
        let state_actions = actions
            .iter()
            .map(action_for_live_state)
            .collect::<Vec<_>>();
        for action in &state_actions {
            self.agent.observe_action(action);
            self.command.observe_action(action);
        }

        let mut links = Vec::new();
        for action in &state_actions {
            links.extend(self.agent.link_pending_for_agent(action));
            links.extend(self.command.link_pending_for_command(action));
        }
        for action in &state_actions {
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
        let update = self.command.observe_process_fork(event);
        let mut links = update.links;
        if let Some(conflict) = update.conflict {
            links.extend(self.agent.invalidate_command_parent_conflict(
                conflict.trace_id,
                &conflict.action_ids,
                &conflict.evidence,
            ));
        }
        links
    }

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.agent.forget_trace(trace_id);
        self.command.forget_trace(trace_id);
        self.sse.forget_trace(trace_id);
    }
}

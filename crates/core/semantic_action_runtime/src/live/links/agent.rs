use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole,
};

use crate::live::actions::ATTR_AGENT_IDENTITY_STATUS;
use crate::live::process_parent::{parent_identity_has_conflict, parent_process_from_action};

use super::shared::{ActionLinkKey, invalidate_child_links, is_nested_file_write_event};

const ATTR_AGENT_ACTION_SEQUENCE: &str = "agent.performed_action.sequence";

#[derive(Default)]
pub(super) struct AgentPerformedActionLinkProjector {
    agents_by_process: BTreeMap<(TraceId, ProcessIdentity), SemanticAction>,
    pending_by_agent_process: BTreeMap<(TraceId, ProcessIdentity), Vec<SemanticAction>>,
    emitted_links: BTreeSet<ActionLinkKey>,
    next_sequence_by_agent: BTreeMap<(TraceId, String), u64>,
}

impl AgentPerformedActionLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) {
        if !is_observed_agent_process(action) {
            return;
        }
        self.agents_by_process
            .insert((action.trace_id, action.process.clone()), action.clone());
    }

    pub(super) fn link_pending_for_agent(
        &mut self,
        agent: &SemanticAction,
    ) -> Vec<SemanticActionLink> {
        if !is_observed_agent_process(agent) {
            return Vec::new();
        }
        let Some(pending) = self
            .pending_by_agent_process
            .remove(&(agent.trace_id, agent.process.clone()))
        else {
            return Vec::new();
        };
        pending
            .iter()
            .filter_map(|action| self.link_agent_child(agent, action))
            .collect()
    }

    pub(super) fn link_child_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        if !agent_performed_action_candidate(action) {
            return Vec::new();
        }
        if parent_identity_has_conflict(action) {
            self.remove_pending_child(action);
            return invalidate_child_links(
                &self.emitted_links,
                action.trace_id,
                &action.action_id,
                SemanticActionLinkRole::AgentPerformedAction,
                &action.evidence,
            );
        }
        let Some(agent) = self.agent_for_child(action).cloned() else {
            self.remember_pending(action);
            return Vec::new();
        };
        self.link_agent_child(&agent, action).into_iter().collect()
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.agents_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_by_agent_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
        self.next_sequence_by_agent
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    fn link_agent_child(
        &mut self,
        agent: &SemanticAction,
        action: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        let parent_action_id = agent.action_id.clone();
        let key = ActionLinkKey {
            trace_id: action.trace_id,
            parent_action_id: parent_action_id.clone(),
            child_action_id: action.action_id.clone(),
            role: SemanticActionLinkRole::AgentPerformedAction,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        let sequence = self.next_sequence(action.trace_id, &parent_action_id);
        let attributes =
            BTreeMap::from([(ATTR_AGENT_ACTION_SEQUENCE.to_string(), sequence.to_string())]);
        Some(SemanticActionLink {
            trace_id: action.trace_id,
            parent_action_id,
            child_action_id: action.action_id.clone(),
            role: SemanticActionLinkRole::AgentPerformedAction,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: action.evidence.clone(),
            attributes,
        })
    }

    fn remember_pending(&mut self, action: &SemanticAction) {
        let Some(agent_process) = candidate_agent_process(action) else {
            return;
        };
        let pending = self
            .pending_by_agent_process
            .entry((action.trace_id, agent_process))
            .or_default();
        if let Some(existing) = pending
            .iter_mut()
            .find(|candidate| candidate.action_id == action.action_id)
        {
            *existing = action.clone();
        } else {
            pending.push(action.clone());
        }
    }

    fn agent_for_child(&self, action: &SemanticAction) -> Option<&SemanticAction> {
        candidate_agent_process(action)
            .and_then(|process| self.agents_by_process.get(&(action.trace_id, process)))
            .filter(|agent| {
                action.kind != SemanticActionKind::CommandInvocation
                    || agent.process != action.process
            })
    }

    fn remove_pending_child(&mut self, action: &SemanticAction) {
        for pending in self.pending_by_agent_process.values_mut() {
            pending.retain(|candidate| candidate.action_id != action.action_id);
        }
        self.pending_by_agent_process
            .retain(|_, pending| !pending.is_empty());
    }

    fn next_sequence(&mut self, trace_id: TraceId, parent_action_id: &str) -> u64 {
        let key = (trace_id, parent_action_id.to_string());
        let next = self.next_sequence_by_agent.entry(key).or_default();
        let sequence = *next;
        *next = next.saturating_add(1);
        sequence
    }
}

fn is_observed_agent_process(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::ProcessExec
        && action
            .attributes
            .get(ATTR_AGENT_IDENTITY_STATUS)
            .is_some_and(|status| status == "observed")
}

fn agent_performed_action_candidate(action: &SemanticAction) -> bool {
    if is_nested_file_write_event(action) {
        return false;
    }
    matches!(
        action.kind,
        SemanticActionKind::LlmCall
            | SemanticActionKind::CommandInvocation
            | SemanticActionKind::FileRead
            | SemanticActionKind::FileWrite
            | SemanticActionKind::FileModify
            | SemanticActionKind::ProcessForkAttempt
    )
}

fn candidate_agent_process(action: &SemanticAction) -> Option<ProcessIdentity> {
    match action.kind {
        SemanticActionKind::LlmCall
        | SemanticActionKind::FileRead
        | SemanticActionKind::FileWrite
        | SemanticActionKind::FileModify
        | SemanticActionKind::ProcessForkAttempt => Some(action.process.clone()),
        SemanticActionKind::CommandInvocation => parent_process_from_action(action),
        _ => None,
    }
}

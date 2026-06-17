use std::collections::{BTreeMap, BTreeSet};

use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole,
};

use super::super::process_parent::{
    ForkProcessEdge, fork_edge_from_event, merge_fork_edges, parent_identity_has_conflict,
    parent_process_from_action,
};
use super::shared::{ActionLinkKey, invalidate_child_links, is_nested_file_write_event};

#[derive(Default)]
pub(super) struct CommandChildActionLinkProjector {
    commands_by_process: BTreeMap<(TraceId, ProcessIdentity), SemanticAction>,
    fork_edges: BTreeMap<(TraceId, ProcessIdentity), ForkProcessEdge>,
    pending_by_process: BTreeMap<(TraceId, ProcessIdentity), Vec<SemanticAction>>,
    emitted_links: BTreeSet<ActionLinkKey>,
}

impl CommandChildActionLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) {
        if action.kind != SemanticActionKind::CommandInvocation {
            return;
        }
        self.commands_by_process
            .insert((action.trace_id, action.process.clone()), action.clone());
    }

    pub(super) fn observe_process_fork(&mut self, event: &DomainEvent) -> Vec<SemanticActionLink> {
        let Some(edge) = fork_edge_from_event(event) else {
            return Vec::new();
        };
        let key = (edge.trace_id, edge.child.clone());
        let edge = merge_fork_edges(self.fork_edges.get(&key), edge);
        self.fork_edges.insert(key.clone(), edge);
        self.link_pending_for_parent_key(&key)
    }

    pub(super) fn link_pending_for_command(
        &mut self,
        command: &SemanticAction,
    ) -> Vec<SemanticActionLink> {
        if command.kind != SemanticActionKind::CommandInvocation {
            return Vec::new();
        }
        self.link_pending_resolved_to_command(command)
    }

    pub(super) fn link_child_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        if !command_child_candidate(action) {
            return Vec::new();
        }
        if parent_identity_has_conflict(action) {
            self.remove_pending_child(action);
            return invalidate_child_links(
                &self.emitted_links,
                action.trace_id,
                &action.action_id,
                SemanticActionLinkRole::CommandContainsCommandInvocation,
                &action.evidence,
            );
        }
        let Some(parent_process) = parent_command_process(action) else {
            return Vec::new();
        };
        let Some(command) = self.command_for_parent(action.trace_id, parent_process.clone()) else {
            self.remember_pending(action);
            return Vec::new();
        };
        self.link(&command, action).into_iter().collect()
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.commands_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.fork_edges
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
    }

    fn link(
        &mut self,
        command: &SemanticAction,
        action: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        if command.action_id == action.action_id {
            return None;
        }
        let role = command_child_role(action)?;
        let key = ActionLinkKey {
            trace_id: action.trace_id,
            parent_action_id: command.action_id.clone(),
            child_action_id: action.action_id.clone(),
            role,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        Some(SemanticActionLink {
            trace_id: action.trace_id,
            parent_action_id: command.action_id.clone(),
            child_action_id: action.action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: action.evidence.clone(),
            attributes: BTreeMap::new(),
        })
    }

    fn remember_pending(&mut self, action: &SemanticAction) {
        let Some(parent_process) = parent_command_process(action) else {
            return;
        };
        let pending = self
            .pending_by_process
            .entry((action.trace_id, parent_process))
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

    fn remove_pending_child(&mut self, action: &SemanticAction) {
        for pending in self.pending_by_process.values_mut() {
            pending.retain(|candidate| candidate.action_id != action.action_id);
        }
        self.pending_by_process
            .retain(|_, pending| !pending.is_empty());
    }

    fn command_for_parent(
        &self,
        trace_id: TraceId,
        parent_process: ProcessIdentity,
    ) -> Option<SemanticAction> {
        let mut candidate = parent_process;
        let mut visited = BTreeSet::new();
        loop {
            if let Some(command) = self
                .commands_by_process
                .get(&(trace_id, candidate.clone()))
                .cloned()
            {
                return Some(command);
            }
            if !visited.insert(candidate.clone()) {
                return None;
            }
            let edge = self.fork_edges.get(&(trace_id, candidate))?;
            if edge.conflict {
                return None;
            }
            candidate = edge.parent.clone()?;
        }
    }

    fn link_pending_for_parent_key(
        &mut self,
        parent_key: &(TraceId, ProcessIdentity),
    ) -> Vec<SemanticActionLink> {
        let Some(parent_command) = self.command_for_parent(parent_key.0, parent_key.1.clone())
        else {
            return Vec::new();
        };
        let Some(pending) = self.pending_by_process.remove(parent_key) else {
            return Vec::new();
        };
        pending
            .iter()
            .filter_map(|action| self.link(&parent_command, action))
            .collect()
    }

    fn link_pending_resolved_to_command(
        &mut self,
        command: &SemanticAction,
    ) -> Vec<SemanticActionLink> {
        let pending_keys = self
            .pending_by_process
            .keys()
            .filter(|(trace_id, parent)| {
                *trace_id == command.trace_id
                    && self
                        .command_for_parent(*trace_id, parent.clone())
                        .is_some_and(|candidate| candidate.action_id == command.action_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut links = Vec::new();
        for key in pending_keys {
            let Some(pending) = self.pending_by_process.remove(&key) else {
                continue;
            };
            for action in pending {
                if let Some(link) = self.link(command, &action) {
                    links.push(link);
                }
            }
        }
        links
    }
}

fn command_child_candidate(action: &SemanticAction) -> bool {
    command_child_role(action).is_some()
}

fn command_child_role(action: &SemanticAction) -> Option<SemanticActionLinkRole> {
    if is_nested_file_write_event(action) {
        return None;
    }
    matches!(
        action.kind,
        SemanticActionKind::FileRead
            | SemanticActionKind::FileWrite
            | SemanticActionKind::FileModify
            | SemanticActionKind::FileTtyIo
            | SemanticActionKind::FileBulkRead
            | SemanticActionKind::FsEnumerate
    )
    .then_some(SemanticActionLinkRole::CommandContainsFileAccess)
    .or_else(|| {
        (action.kind == SemanticActionKind::ProcessForkAttempt)
            .then_some(SemanticActionLinkRole::CommandContainsProcessForkAttempt)
    })
    .or_else(|| {
        (action.kind == SemanticActionKind::CommandInvocation)
            .then_some(SemanticActionLinkRole::CommandContainsCommandInvocation)
    })
    .or_else(|| {
        (action.kind == SemanticActionKind::LlmCall)
            .then_some(SemanticActionLinkRole::CommandContainsLlmCall)
    })
    .or_else(|| {
        (action.kind == SemanticActionKind::AgentInvocation)
            .then_some(SemanticActionLinkRole::CommandContainsCommandInvocation)
    })
}

fn parent_command_process(action: &SemanticAction) -> Option<ProcessIdentity> {
    match action.kind {
        SemanticActionKind::FileRead
        | SemanticActionKind::FileWrite
        | SemanticActionKind::FileModify
        | SemanticActionKind::FileTtyIo
        | SemanticActionKind::FileBulkRead
        | SemanticActionKind::FsEnumerate
        | SemanticActionKind::ProcessForkAttempt
        | SemanticActionKind::LlmCall
        | SemanticActionKind::AgentInvocation => Some(action.process.clone()),
        SemanticActionKind::CommandInvocation => parent_process_from_action(action),
        _ => None,
    }
}

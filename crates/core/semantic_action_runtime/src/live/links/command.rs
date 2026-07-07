use std::collections::{BTreeMap, BTreeSet};

use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole, attr_keys as attrs,
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
    fork_children: BTreeMap<(TraceId, ProcessIdentity), BTreeSet<ProcessIdentity>>,
    pending_by_process: BTreeMap<(TraceId, ProcessIdentity), Vec<SemanticAction>>,
    pending_key_by_child: BTreeMap<(TraceId, String), (TraceId, ProcessIdentity)>,
    pending_owner_candidates:
        BTreeMap<(TraceId, ProcessIdentity), BTreeSet<(TraceId, ProcessIdentity)>>,
    pending_candidate_owners:
        BTreeMap<(TraceId, ProcessIdentity), BTreeSet<(TraceId, ProcessIdentity)>>,
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
        let previous = self.fork_edges.get(&key).cloned();
        let edge = merge_fork_edges(self.fork_edges.get(&key), edge);
        self.fork_edges.insert(key.clone(), edge);
        let merged = self.fork_edges.get(&key).cloned();
        self.update_fork_child_index(previous.as_ref(), merged.as_ref());
        let affected_pending_keys = self.pending_keys_for_process_subtree(&key);
        for pending_key in &affected_pending_keys {
            self.refresh_pending_owner_candidates(pending_key);
        }
        affected_pending_keys
            .iter()
            .flat_map(|pending_key| self.link_pending_for_parent_key(pending_key))
            .collect()
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
            let Some(role) = command_child_role(action) else {
                return Vec::new();
            };
            return invalidate_child_links(
                &self.emitted_links,
                action.trace_id,
                &action.action_id,
                role,
                &action.evidence,
            );
        }
        let Some(command) = self.command_for_child_action(action) else {
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
        self.fork_children
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_by_process
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_key_by_child
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_owner_candidates
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.pending_candidate_owners
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
            valid: true,
            evidence: action.evidence.clone(),
            attributes: BTreeMap::new(),
        })
    }

    fn remember_pending(&mut self, action: &SemanticAction) {
        let Some(parent_process) = parent_command_process(action) else {
            return;
        };
        let child_key = (action.trace_id, action.action_id.clone());
        if let Some(previous_key) = self.pending_key_by_child.remove(&child_key)
            && previous_key != (action.trace_id, parent_process.clone())
            && let Some(pending) = self.pending_by_process.get_mut(&previous_key)
        {
            pending.retain(|candidate| candidate.action_id != action.action_id);
        }
        let pending_key = (action.trace_id, parent_process.clone());
        let pending = self
            .pending_by_process
            .entry(pending_key.clone())
            .or_default();
        if let Some(existing) = pending
            .iter_mut()
            .find(|candidate| candidate.action_id == action.action_id)
        {
            *existing = action.clone();
        } else {
            pending.push(action.clone());
        }
        self.pending_key_by_child
            .insert(child_key, (action.trace_id, parent_process));
        self.refresh_pending_owner_candidates(&pending_key);
    }

    fn remove_pending_child(&mut self, action: &SemanticAction) {
        let child_key = (action.trace_id, action.action_id.clone());
        let Some(pending_key) = self.pending_key_by_child.remove(&child_key) else {
            return;
        };
        if let Some(pending) = self.pending_by_process.get_mut(&pending_key) {
            pending.retain(|candidate| candidate.action_id != action.action_id);
        }
        if self
            .pending_by_process
            .get(&pending_key)
            .is_some_and(Vec::is_empty)
        {
            self.pending_by_process.remove(&pending_key);
            self.remove_pending_owner_candidates(&pending_key);
        }
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

    fn command_for_child_action(&self, action: &SemanticAction) -> Option<SemanticAction> {
        if action.kind == SemanticActionKind::McpToolCall {
            if let Some(parent_process) = parent_process_from_action(action) {
                return self.command_for_parent(action.trace_id, parent_process);
            }
            return self.command_for_mcp_client_pid(action);
        }
        let parent_process = parent_command_process(action)?;
        self.command_for_parent(action.trace_id, parent_process)
    }

    fn command_for_mcp_client_pid(&self, action: &SemanticAction) -> Option<SemanticAction> {
        let client_pid = action
            .attributes
            .get(attrs::mcp::CLIENT_PID)?
            .parse::<u32>()
            .ok()?;
        let mut candidates = self
            .commands_by_process
            .iter()
            .filter(|((trace_id, process), command)| {
                *trace_id == action.trace_id
                    && process.pid == client_pid
                    && command.start_time <= action.start_time
                    && command
                        .end_time
                        .is_none_or(|end_time| action.start_time <= end_time)
            })
            .map(|(_, command)| command.clone())
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            candidates.pop()
        } else {
            None
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
        self.link_pending_to_command(parent_key, &parent_command)
    }

    fn link_pending_to_command(
        &mut self,
        parent_key: &(TraceId, ProcessIdentity),
        parent_command: &SemanticAction,
    ) -> Vec<SemanticActionLink> {
        let Some(pending) = self.pending_by_process.remove(parent_key) else {
            return Vec::new();
        };
        self.remove_pending_owner_candidates(parent_key);
        pending
            .iter()
            .filter_map(|action| {
                self.pending_key_by_child
                    .remove(&(action.trace_id, action.action_id.clone()));
                self.link(&parent_command, action)
            })
            .collect()
    }

    fn link_pending_resolved_to_command(
        &mut self,
        command: &SemanticAction,
    ) -> Vec<SemanticActionLink> {
        let owner_key = (command.trace_id, command.process.clone());
        let pending_keys = self
            .pending_owner_candidates
            .get(&owner_key)
            .cloned()
            .unwrap_or_default();
        let mut links = Vec::new();
        for key in pending_keys {
            let Some(candidate) = self.command_for_parent(key.0, key.1.clone()) else {
                continue;
            };
            if candidate.action_id != command.action_id {
                continue;
            }
            links.extend(self.link_pending_to_command(&key, command));
        }
        links
    }

    fn update_fork_child_index(
        &mut self,
        previous: Option<&ForkProcessEdge>,
        current: Option<&ForkProcessEdge>,
    ) {
        if let Some(edge) = previous
            && let Some(parent) = &edge.parent
        {
            let parent_key = (edge.trace_id, parent.clone());
            if let Some(children) = self.fork_children.get_mut(&parent_key) {
                children.remove(&edge.child);
                if children.is_empty() {
                    self.fork_children.remove(&parent_key);
                }
            }
        }
        let Some(edge) = current else {
            return;
        };
        if edge.conflict {
            return;
        }
        if let Some(parent) = &edge.parent {
            self.fork_children
                .entry((edge.trace_id, parent.clone()))
                .or_default()
                .insert(edge.child.clone());
        }
    }

    fn pending_keys_for_process_subtree(
        &self,
        root: &(TraceId, ProcessIdentity),
    ) -> BTreeSet<(TraceId, ProcessIdentity)> {
        let mut pending_keys = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut stack = vec![root.1.clone()];
        while let Some(process) = stack.pop() {
            if !visited.insert(process.clone()) {
                continue;
            }
            let key = (root.0, process.clone());
            if self.pending_by_process.contains_key(&key) {
                pending_keys.insert(key.clone());
            }
            if let Some(children) = self.fork_children.get(&key) {
                stack.extend(children.iter().cloned());
            }
        }
        pending_keys
    }

    fn refresh_pending_owner_candidates(&mut self, pending_key: &(TraceId, ProcessIdentity)) {
        self.remove_pending_owner_candidates(pending_key);
        if !self.pending_by_process.contains_key(pending_key) {
            return;
        }
        let owners = self.owner_candidates_for_pending_key(pending_key);
        for owner in &owners {
            self.pending_owner_candidates
                .entry(owner.clone())
                .or_default()
                .insert(pending_key.clone());
        }
        self.pending_candidate_owners
            .insert(pending_key.clone(), owners);
    }

    fn remove_pending_owner_candidates(&mut self, pending_key: &(TraceId, ProcessIdentity)) {
        let Some(owners) = self.pending_candidate_owners.remove(pending_key) else {
            return;
        };
        for owner in owners {
            if let Some(keys) = self.pending_owner_candidates.get_mut(&owner) {
                keys.remove(pending_key);
                if keys.is_empty() {
                    self.pending_owner_candidates.remove(&owner);
                }
            }
        }
    }

    fn owner_candidates_for_pending_key(
        &self,
        pending_key: &(TraceId, ProcessIdentity),
    ) -> BTreeSet<(TraceId, ProcessIdentity)> {
        let mut owners = BTreeSet::new();
        let mut candidate = pending_key.1.clone();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(candidate.clone()) {
                break;
            }
            owners.insert((pending_key.0, candidate.clone()));
            let Some(edge) = self.fork_edges.get(&(pending_key.0, candidate)) else {
                break;
            };
            if edge.conflict {
                break;
            }
            let Some(parent) = edge.parent.clone() else {
                break;
            };
            candidate = parent;
        }
        owners
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
        (action.kind == SemanticActionKind::McpToolCall)
            .then_some(SemanticActionLinkRole::CommandContainsMcpToolCall)
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
        SemanticActionKind::McpToolCall => parent_process_from_action(action),
        SemanticActionKind::CommandInvocation => parent_process_from_action(action),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    use model_core::ids::TraceId;
    use model_core::process::ProcessIdentity;
    use semantic_action::{
        SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole,
        SemanticActionStatus, attr_keys as attrs,
    };

    use super::CommandChildActionLinkProjector;

    #[test]
    fn mcp_tool_call_links_to_command_by_client_pid_without_parent_identity() {
        let mut projector = CommandChildActionLinkProjector::default();
        let client = ProcessIdentity::new(5100, 9100, 10_100);
        let server = ProcessIdentity::new(5101, 9101, 10_101);
        let command = action(
            "client-command",
            SemanticActionKind::CommandInvocation,
            client.clone(),
            1,
        );
        let mut mcp = action("mcp-tool", SemanticActionKind::McpToolCall, server, 2);
        mcp.attributes
            .insert(attrs::mcp::CLIENT_PID.to_string(), client.pid.to_string());

        projector.observe_action(&command);
        let links = projector.link_child_action(&mcp);

        assert!(links.iter().any(|link| {
            link.role == SemanticActionLinkRole::CommandContainsMcpToolCall
                && link.parent_action_id == command.action_id
                && link.child_action_id == mcp.action_id
        }));
    }

    #[test]
    fn mcp_tool_call_does_not_link_by_unmatched_client_pid() {
        let mut projector = CommandChildActionLinkProjector::default();
        let client = ProcessIdentity::new(5200, 9200, 10_200);
        let server = ProcessIdentity::new(5201, 9201, 10_201);
        let command = action(
            "client-command",
            SemanticActionKind::CommandInvocation,
            client,
            1,
        );
        let mut mcp = action("mcp-tool", SemanticActionKind::McpToolCall, server, 2);
        mcp.attributes
            .insert(attrs::mcp::CLIENT_PID.to_string(), "5299".to_string());

        projector.observe_action(&command);
        let links = projector.link_child_action(&mcp);

        assert!(links.iter().all(|link| {
            link.role != SemanticActionLinkRole::CommandContainsMcpToolCall
                || link.child_action_id != mcp.action_id
        }));
    }

    fn action(
        action_id: &str,
        kind: SemanticActionKind,
        process: ProcessIdentity,
        start_millis: u64,
    ) -> SemanticAction {
        SemanticAction {
            action_id: action_id.to_string(),
            trace_id: TraceId::new(1),
            kind,
            title: action_id.to_string(),
            start_time: UNIX_EPOCH + Duration::from_millis(start_millis),
            end_time: None,
            process,
            status: SemanticActionStatus::InProgress,
            completeness: SemanticActionCompleteness::Partial,
            confidence_millis: None,
            attributes: BTreeMap::new(),
            evidence: Vec::new(),
        }
    }
}

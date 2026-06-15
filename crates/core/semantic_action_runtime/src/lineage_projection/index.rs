use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership};
use semantic_action::{SemanticAction, SemanticActionKind};

use super::ATTR_AGENT_IDENTITY_STATUS;

pub(super) struct LineageIndex<'a> {
    actions: Vec<&'a SemanticAction>,
    parent_by_process: BTreeMap<ProcessIdentity, ProcessIdentity>,
    commands_by_process: BTreeMap<ProcessIdentity, Vec<&'a SemanticAction>>,
    agents_by_process: BTreeMap<ProcessIdentity, Vec<&'a SemanticAction>>,
}

impl<'a> LineageIndex<'a> {
    pub(super) fn new(
        trace_id: TraceId,
        memberships: &'a [ProcessMembership],
        actions: &'a [SemanticAction],
    ) -> Self {
        let mut parent_by_process = BTreeMap::new();
        for membership in memberships
            .iter()
            .filter(|membership| membership.trace_id == trace_id)
        {
            if let Some(parent) = &membership.inherited_from {
                parent_by_process.insert(membership.identity.clone(), parent.clone());
            }
        }

        let mut commands_by_process = BTreeMap::<ProcessIdentity, Vec<&SemanticAction>>::new();
        let mut agents_by_process = BTreeMap::<ProcessIdentity, Vec<&SemanticAction>>::new();
        let mut filtered_actions = actions
            .iter()
            .filter(|action| action.trace_id == trace_id)
            .collect::<Vec<_>>();
        filtered_actions.sort_by(|left, right| {
            (left.start_time, left.action_id.as_str())
                .cmp(&(right.start_time, right.action_id.as_str()))
        });
        for action in &filtered_actions {
            if action.kind == SemanticActionKind::CommandInvocation {
                commands_by_process
                    .entry(action.process.clone())
                    .or_default()
                    .push(*action);
            }
            if is_observed_agent(action) {
                agents_by_process
                    .entry(action.process.clone())
                    .or_default()
                    .push(*action);
            }
        }

        Self {
            actions: filtered_actions,
            parent_by_process,
            commands_by_process,
            agents_by_process,
        }
    }

    pub(super) fn actions(&self) -> &[&'a SemanticAction] {
        &self.actions
    }

    pub(super) fn action_by_id(&self) -> BTreeMap<String, &'a SemanticAction> {
        self.actions
            .iter()
            .map(|action| (action.action_id.clone(), *action))
            .collect()
    }

    pub(super) fn parent_process(&self, process: &ProcessIdentity) -> Option<ProcessIdentity> {
        self.parent_by_process.get(process).cloned()
    }

    pub(super) fn nearest_command(
        &self,
        start: &ProcessIdentity,
        at: SystemTime,
        child_action_id: &str,
    ) -> Option<&'a SemanticAction> {
        let mut current = start.clone();
        let mut visited = BTreeSet::new();
        loop {
            if let Some(command) = self.command_for_process(&current, at, child_action_id) {
                return Some(command);
            }
            if !visited.insert(current.clone()) {
                return None;
            }
            current = self.parent_by_process.get(&current)?.clone();
        }
    }

    pub(super) fn agent_for_process(
        &self,
        process: &ProcessIdentity,
        at: SystemTime,
    ) -> Option<&'a SemanticAction> {
        self.agents_by_process
            .get(process)?
            .iter()
            .rev()
            .find(|agent| {
                agent.start_time <= at && agent.end_time.is_none_or(|end_time| at <= end_time)
            })
            .copied()
    }

    fn command_for_process(
        &self,
        process: &ProcessIdentity,
        at: SystemTime,
        child_action_id: &str,
    ) -> Option<&'a SemanticAction> {
        self.commands_by_process
            .get(process)?
            .iter()
            .rev()
            .find(|command| {
                command.action_id != child_action_id
                    && command.start_time <= at
                    && command.end_time.is_none_or(|end_time| at <= end_time)
            })
            .copied()
    }
}

fn is_observed_agent(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::ProcessExec
        && action
            .attributes
            .get(ATTR_AGENT_IDENTITY_STATUS)
            .is_some_and(|status| status == "observed")
}

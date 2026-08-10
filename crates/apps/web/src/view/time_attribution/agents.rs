use super::*;

#[derive(Default)]
pub(super) struct AgentProcessScope {
    pub(super) subjects: BTreeSet<ProcessIdentity>,
    pub(super) subtree: BTreeSet<ProcessIdentity>,
    invocation_wrappers: BTreeSet<ProcessIdentity>,
    first_model_start: BTreeMap<ProcessIdentity, u128>,
    parent_by_process: BTreeMap<ProcessIdentity, ProcessIdentity>,
}

impl AgentProcessScope {
    pub(super) fn is_agent_runtime_command(&self, action: &SemanticAction) -> bool {
        if self.invocation_wrappers.contains(&action.process) {
            return true;
        }
        let Some(first_model_start) = self.first_model_start.get(&action.process) else {
            return false;
        };
        system_time_nanos(action.start_time)
            .is_ok_and(|command_start| command_start <= *first_model_start)
    }

    pub(super) fn is_agent_runtime_process(&self, process: ProcessIdentity) -> bool {
        self.subjects.contains(&process) || self.invocation_wrappers.contains(&process)
    }

    fn nearest_agent_ancestor(&self, process: ProcessIdentity) -> Option<ProcessIdentity> {
        let mut cursor = process;
        let mut visited = BTreeSet::new();
        while visited.insert(cursor) {
            let parent = *self.parent_by_process.get(&cursor)?;
            if self.subjects.contains(&parent) {
                return Some(parent);
            }
            cursor = parent;
        }
        None
    }

    fn is_same_or_ancestor(&self, candidate: ProcessIdentity, process: ProcessIdentity) -> bool {
        let mut cursor = process;
        let mut visited = BTreeSet::new();
        loop {
            if cursor == candidate {
                return true;
            }
            if !visited.insert(cursor) {
                return false;
            }
            let Some(parent) = self.parent_by_process.get(&cursor).copied() else {
                return false;
            };
            cursor = parent;
        }
    }
}

pub(super) fn agent_process_scope(
    memberships: &[ProcessMembership],
    calls: &[ModelInterval],
    tracker: &mut StatusTracker,
) -> AgentProcessScope {
    let mut evidence_start = BTreeMap::<ProcessIdentity, u128>::new();
    for call in calls {
        evidence_start
            .entry(call.process)
            .and_modify(|current| *current = (*current).min(call.interval.start))
            .or_insert(call.interval.start);
    }
    if evidence_start.is_empty() && !calls.is_empty() {
        tracker.warning(
            "agent_identity_missing",
            "Model calls exist but no reliable Agent process identity was found.",
        );
    }

    let subjects = evidence_start.keys().copied().collect::<BTreeSet<_>>();
    let mut subtree = subjects.clone();
    loop {
        let before = subtree.len();
        for membership in memberships {
            if membership
                .inherited_from
                .as_ref()
                .is_some_and(|parent| subtree.contains(parent))
            {
                subtree.insert(membership.identity);
            }
        }
        if before == subtree.len() {
            break;
        }
    }

    let membership_by_process = memberships
        .iter()
        .map(|membership| (membership.identity, membership))
        .collect::<BTreeMap<_, _>>();
    for process in &subjects {
        let Some(membership) = membership_by_process.get(process).copied() else {
            tracker.warning(
                "agent_membership_missing",
                format!(
                    "Confirmed Agent process {process} has no membership boundary; only directly bounded local actions are attributable."
                ),
            );
            continue;
        };
        if membership.state == MembershipState::IdentityStale {
            tracker.warning(
                "agent_identity_stale",
                format!("Confirmed Agent process {process} has stale identity coordinates."),
            );
        }
    }
    if subjects.is_empty() && !calls.is_empty() {
        tracker.warning(
            "agent_identity_missing",
            "No confirmed Agent process was observed; non-model time is left unattributed.",
        );
    }

    let parent_by_process = memberships
        .iter()
        .filter_map(|membership| {
            membership
                .inherited_from
                .map(|parent| (membership.identity, parent))
        })
        .collect::<BTreeMap<_, _>>();
    let mut invocation_wrappers = BTreeSet::new();
    for subject in &subjects {
        let mut cursor = *subject;
        let mut visited = BTreeSet::new();
        while visited.insert(cursor) {
            let Some(parent) = parent_by_process.get(&cursor).copied() else {
                break;
            };
            if subjects.contains(&parent) {
                break;
            }
            invocation_wrappers.insert(parent);
            cursor = parent;
        }
    }

    AgentProcessScope {
        subjects,
        subtree,
        invocation_wrappers,
        first_model_start: evidence_start,
        parent_by_process,
    }
}

pub(super) fn include_nested_agent_calls(
    observed_calls: &[ModelInterval],
    mut accepted_calls: Vec<ModelInterval>,
    turns: &mut [UserTurn],
    tools: &[ToolInterval],
    agent_scope: &AgentProcessScope,
    tracker: &mut StatusTracker,
) -> Vec<ModelInterval> {
    let mut accepted_ids = accepted_calls
        .iter()
        .map(|call| call.action_id.clone())
        .collect::<BTreeSet<_>>();
    let accepted_process_by_id = accepted_calls
        .iter()
        .map(|call| (call.action_id.clone(), call.process))
        .collect::<BTreeMap<_, _>>();
    let mut turns_by_process = BTreeMap::<ProcessIdentity, BTreeSet<usize>>::new();
    for (turn_index, turn) in turns.iter().enumerate() {
        for action_id in &turn.call_action_ids {
            let Some(process) = accepted_process_by_id.get(action_id) else {
                continue;
            };
            turns_by_process
                .entry(*process)
                .or_default()
                .insert(turn_index);
        }
    }

    let mut pending = observed_calls
        .iter()
        .filter(|call| !accepted_ids.contains(&call.action_id))
        .cloned()
        .collect::<Vec<_>>();
    loop {
        let mut next_pending = Vec::new();
        let mut added = false;
        for call in pending {
            let Some(parent_agent) = agent_scope.nearest_agent_ancestor(call.process) else {
                next_pending.push(call);
                continue;
            };
            let Some(turn_indexes) = turns_by_process.get(&parent_agent) else {
                next_pending.push(call);
                continue;
            };
            let turn_index = turn_indexes.iter().copied().find(|turn_index| {
                let turn = &turns[*turn_index];
                call.interval.intersect(turn.interval).is_some()
                    && tools.iter().any(|tool| {
                        tool.agent_invocation
                            && tool.interval.start >= turn.interval.start
                            && tool.interval.start <= call.interval.start
                            && tool.interval.end > call.interval.start
                            && agent_scope.is_same_or_ancestor(tool.process, call.process)
                    })
            });
            let Some(turn_index) = turn_index else {
                next_pending.push(call);
                continue;
            };

            turns[turn_index]
                .call_action_ids
                .push(call.action_id.clone());
            turns_by_process
                .entry(call.process)
                .or_default()
                .insert(turn_index);
            accepted_ids.insert(call.action_id.clone());
            tracker.action_info(
                "nested_agent_llm_call_included",
                "LLM call belongs to an Agent runtime invoked during the user request and is included as model-side observable time.",
                &call.action_id,
                Some(call.interval),
            );
            accepted_calls.push(call);
            added = true;
        }
        if !added {
            break;
        }
        pending = next_pending;
    }

    accepted_calls.sort_by(|left, right| {
        (left.interval.start, left.interval.end, &left.action_id).cmp(&(
            right.interval.start,
            right.interval.end,
            &right.action_id,
        ))
    });
    accepted_calls
}

//! Display projection for the web action tree.

use std::collections::{BTreeMap, BTreeSet};

use model_core::{ids::TraceId, process::ProcessIdentity};
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
    SemanticEvidenceKind, attr_keys as attrs, evidence_roles,
};
use storage_core::StorageBackend;

use super::action_tree_roles::{DISPLAY_PARENT_ROLES, ROOT_LINK_ROLES};

#[path = "action_tree_projection/legacy_llm_call.rs"]
mod legacy_llm_call;
#[path = "action_tree_projection/link_validity.rs"]
mod link_validity;

const ACTION_VALID_ATTR: &str = "actrail.action.valid";
const VALID_FALSE: &str = "false";
pub(super) const ROOT_PARENT_ID: &str = "";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisplayChild {
    pub action: SemanticAction,
    pub link: Option<SemanticActionLink>,
    pub child_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActionDisplayProjection {
    pub actions: Vec<SemanticAction>,
    pub links: Vec<SemanticActionLink>,
    pub root_action_ids: Vec<String>,
    children_by_parent: BTreeMap<String, Vec<DisplayChild>>,
}

impl ActionDisplayProjection {
    pub(super) fn load(
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
    ) -> Result<Self, String> {
        let actions = load_semantic_actions(storage, trace_id)?;
        let links = load_semantic_action_links(storage, trace_id)?;
        Ok(Self::new(actions, links))
    }

    #[cfg(test)]
    pub(super) fn children(&self, parent_id: &str) -> Vec<DisplayChild> {
        self.children_by_parent
            .get(parent_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn child_count(&self, parent_id: &str) -> usize {
        self.children_by_parent
            .get(parent_id)
            .map(Vec::len)
            .unwrap_or_default()
    }

    pub(super) fn children_page(
        &self,
        parent_id: &str,
        offset: usize,
        limit: usize,
    ) -> (Vec<DisplayChild>, usize) {
        let Some(children) = self.children_by_parent.get(parent_id) else {
            return (Vec::new(), 0);
        };
        let total = children.len();
        let page = children.iter().skip(offset).take(limit).cloned().collect();
        (page, total)
    }

    fn new(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        let actions = valid_actions(actions);
        let action_by_id = actions
            .iter()
            .map(|action| (action.action_id.clone(), action.clone()))
            .collect::<BTreeMap<_, _>>();
        let links = valid_links(links, &action_by_id);
        let mut actions = legacy_llm_call::normalize_finalized_http_error_calls(actions, &links);
        let action_by_id = actions
            .iter()
            .map(|action| (action.action_id.clone(), action.clone()))
            .collect::<BTreeMap<_, _>>();
        let links = valid_links(links, &action_by_id);
        let parent_links = selected_parent_links(&links);
        let root_links = selected_root_links(&links);
        let mut parent_links = remove_cycles(parent_links);
        fold_triggered_vfork_attempts(&mut actions, &mut parent_links);
        let action_by_id = actions
            .iter()
            .map(|action| (action.action_id.clone(), action.clone()))
            .collect::<BTreeMap<_, _>>();
        let fallback_parents = fallback_display_parents(&actions, &parent_links);
        let children_by_parent = children_by_parent(
            &actions,
            &parent_links,
            &fallback_parents,
            &root_links,
            &action_by_id,
        );
        let root_action_ids = children_by_parent
            .get(ROOT_PARENT_ID)
            .map(|children| {
                children
                    .iter()
                    .map(|child| child.action.action_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let display_links = display_links(&parent_links, &root_links, &root_action_ids);
        let mut projection = Self {
            actions,
            links: display_links,
            root_action_ids,
            children_by_parent,
        };
        projection.attach_child_counts();
        projection
    }

    fn attach_child_counts(&mut self) {
        let child_counts = self
            .children_by_parent
            .iter()
            .map(|(parent, children)| (parent.clone(), children.len()))
            .collect::<BTreeMap<_, _>>();
        for children in self.children_by_parent.values_mut() {
            for child in children {
                child.child_count = child_counts
                    .get(&child.action.action_id)
                    .copied()
                    .unwrap_or_default();
            }
        }
    }
}

fn load_semantic_actions(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<Vec<SemanticAction>, String> {
    let mut actions = storage.list_semantic_actions(trace_id).map_err(|error| {
        format!(
            "list semantic actions failed: {}: {}",
            error.stage, error.message
        )
    })?;
    actions.sort_by(|left, right| {
        (left.start_time, left.action_id.as_str())
            .cmp(&(right.start_time, right.action_id.as_str()))
    });
    actions.dedup_by(|left, right| left.action_id == right.action_id);
    Ok(actions)
}

fn load_semantic_action_links(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<Vec<SemanticActionLink>, String> {
    storage
        .list_semantic_action_links(trace_id)
        .map_err(|error| {
            format!(
                "list semantic action links failed: {}: {}",
                error.stage, error.message
            )
        })
}

fn valid_actions(actions: Vec<SemanticAction>) -> Vec<SemanticAction> {
    let actions = actions
        .into_iter()
        .filter(|action| !invalidated_action(action))
        .collect::<Vec<_>>();
    let bulk_read_ranges = bulk_read_ranges(&actions);
    actions
        .into_iter()
        .filter(|action| !bulk_read_covered_file_read(action, &bulk_read_ranges))
        .collect()
}

fn valid_links(
    links: Vec<SemanticActionLink>,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> Vec<SemanticActionLink> {
    links
        .into_iter()
        .filter(|link| {
            let Some(parent) = action_by_id.get(&link.parent_action_id) else {
                return false;
            };
            let Some(child) = action_by_id.get(&link.child_action_id) else {
                return false;
            };
            !link_validity::invalid_link(link, parent, child, action_by_id)
        })
        .collect()
}

fn selected_parent_links(links: &[SemanticActionLink]) -> BTreeMap<String, SemanticActionLink> {
    let mut selected = BTreeMap::<String, SemanticActionLink>::new();
    for link in links
        .iter()
        .filter(|link| DISPLAY_PARENT_ROLES.contains(&link.role))
    {
        match selected.get(&link.child_action_id) {
            Some(current) if display_role_order(current.role) <= display_role_order(link.role) => {}
            _ => {
                selected.insert(link.child_action_id.clone(), link.clone());
            }
        }
    }
    selected
}

fn selected_root_links(links: &[SemanticActionLink]) -> BTreeMap<String, SemanticActionLink> {
    let mut selected = BTreeMap::<String, SemanticActionLink>::new();
    for link in links
        .iter()
        .filter(|link| ROOT_LINK_ROLES.contains(&link.role))
    {
        selected
            .entry(link.child_action_id.clone())
            .or_insert_with(|| link.clone());
    }
    selected
}

fn remove_cycles(
    parent_links: BTreeMap<String, SemanticActionLink>,
) -> BTreeMap<String, SemanticActionLink> {
    parent_links
        .iter()
        .filter(|(child_id, link)| {
            !parent_path_reaches_child(child_id, &link.parent_action_id, &parent_links)
        })
        .map(|(child_id, link)| (child_id.clone(), link.clone()))
        .collect()
}

fn parent_path_reaches_child(
    child_id: &str,
    parent_id: &str,
    parent_links: &BTreeMap<String, SemanticActionLink>,
) -> bool {
    let mut current = parent_id;
    let mut visited = BTreeSet::new();
    while let Some(link) = parent_links.get(current) {
        if link.parent_action_id == child_id {
            return true;
        }
        if !visited.insert(current.to_string()) {
            return true;
        }
        current = &link.parent_action_id;
    }
    false
}

fn fold_triggered_vfork_attempts(
    actions: &mut [SemanticAction],
    parent_links: &mut BTreeMap<String, SemanticActionLink>,
) {
    let action_index_by_id = actions
        .iter()
        .enumerate()
        .map(|(index, action)| (action.action_id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut children_by_parent = BTreeMap::<String, Vec<String>>::new();
    for (child_id, link) in parent_links.iter() {
        if action_index_by_id.contains_key(child_id) {
            children_by_parent
                .entry(link.parent_action_id.clone())
                .or_default()
                .push(child_id.clone());
        }
    }
    for child_ids in children_by_parent.values_mut() {
        child_ids.sort_by(|left, right| {
            let left = &actions[action_index_by_id[left]];
            let right = &actions[action_index_by_id[right]];
            (left.start_time, left.action_id.as_str())
                .cmp(&(right.start_time, right.action_id.as_str()))
        });
        let mut pending_vforks = Vec::<String>::new();
        for child_id in child_ids {
            let action_index = action_index_by_id[child_id];
            let action = &actions[action_index];
            if foldable_vfork_attempt(action) {
                pending_vforks.push(child_id.clone());
                continue;
            }
            if action.kind != SemanticActionKind::CommandInvocation
                || !has_process_fork_evidence(action)
            {
                continue;
            }
            let Some(parent_process) = parent_process_from_action(action) else {
                continue;
            };
            let Some(pending_index) = pending_vforks.iter().rposition(|fork_id| {
                let fork_index = action_index_by_id[fork_id];
                actions[fork_index].process == parent_process
            }) else {
                continue;
            };
            let fork_id = pending_vforks.remove(pending_index);
            let fork_index = action_index_by_id[&fork_id];
            let fork_start_time = actions[fork_index].start_time;
            if let Some(link) = parent_links.get_mut(&fork_id) {
                link.parent_action_id = child_id.clone();
            }
            if fork_start_time < actions[action_index].start_time {
                actions[action_index].start_time = fork_start_time;
            }
        }
    }
}

fn foldable_vfork_attempt(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::ProcessForkAttempt
        && action
            .attributes
            .get("syscall")
            .is_some_and(|syscall| syscall == "vfork")
}

fn has_process_fork_evidence(action: &SemanticAction) -> bool {
    action
        .evidence
        .iter()
        .any(|evidence| evidence.role == evidence_roles::process::FORK)
}

fn parent_process_from_action(action: &SemanticAction) -> Option<ProcessIdentity> {
    if !action
        .attributes
        .get(attrs::process_parent::IDENTITY_STATE)
        .is_some_and(|state| state == "observed")
    {
        return None;
    }
    parse_u64_attr(action, attrs::process_parent::ID).map(ProcessIdentity::new)
}

fn parse_u64_attr(action: &SemanticAction, key: &str) -> Option<u64> {
    action.attributes.get(key)?.parse().ok()
}

fn children_by_parent(
    actions: &[SemanticAction],
    parent_links: &BTreeMap<String, SemanticActionLink>,
    fallback_parents: &BTreeMap<String, String>,
    root_links: &BTreeMap<String, SemanticActionLink>,
    action_by_id: &BTreeMap<String, SemanticAction>,
) -> BTreeMap<String, Vec<DisplayChild>> {
    let mut children = BTreeMap::<String, Vec<DisplayChild>>::new();
    for action in actions {
        let (parent_id, link) = parent_links
            .get(&action.action_id)
            .filter(|link| action_by_id.contains_key(&link.parent_action_id))
            .map(|link| (link.parent_action_id.clone(), Some(link.clone())))
            .or_else(|| {
                fallback_parents
                    .get(&action.action_id)
                    .filter(|parent_id| action_by_id.contains_key(parent_id.as_str()))
                    .map(|parent_id| (parent_id.clone(), None))
            })
            .unwrap_or_else(|| {
                (
                    ROOT_PARENT_ID.to_string(),
                    root_links.get(&action.action_id).cloned(),
                )
            });
        children.entry(parent_id).or_default().push(DisplayChild {
            action: action.clone(),
            link,
            child_count: 0,
        });
    }
    for rows in children.values_mut() {
        rows.sort_by(|left, right| {
            (left.action.start_time, left.action.action_id.as_str())
                .cmp(&(right.action.start_time, right.action.action_id.as_str()))
        });
    }
    children
}

fn fallback_display_parents(
    actions: &[SemanticAction],
    parent_links: &BTreeMap<String, SemanticActionLink>,
) -> BTreeMap<String, String> {
    let commands_by_process = commands_by_process(actions);
    let mut fallback_parents = BTreeMap::new();
    for action in actions {
        if parent_links.contains_key(&action.action_id)
            || action.kind == SemanticActionKind::CommandInvocation
        {
            continue;
        }
        let Some(parent) = command_for_action(action, &commands_by_process) else {
            continue;
        };
        if parent.action_id != action.action_id
            && !parent_path_reaches_child(&action.action_id, &parent.action_id, parent_links)
        {
            fallback_parents.insert(action.action_id.clone(), parent.action_id.clone());
        }
    }
    fallback_parents
}

fn commands_by_process(
    actions: &[SemanticAction],
) -> BTreeMap<ProcessIdentity, Vec<&SemanticAction>> {
    let mut commands = BTreeMap::<ProcessIdentity, Vec<&SemanticAction>>::new();
    for action in actions {
        if action.kind == SemanticActionKind::CommandInvocation {
            commands
                .entry(action.process.clone())
                .or_default()
                .push(action);
        }
    }
    for rows in commands.values_mut() {
        rows.sort_by(|left, right| {
            (left.start_time, left.action_id.as_str())
                .cmp(&(right.start_time, right.action_id.as_str()))
        });
    }
    commands
}

fn command_for_action<'a>(
    action: &SemanticAction,
    commands_by_process: &'a BTreeMap<ProcessIdentity, Vec<&'a SemanticAction>>,
) -> Option<&'a SemanticAction> {
    commands_by_process
        .get(&action.process)?
        .iter()
        .rev()
        .find(|command| {
            command.start_time <= action.start_time
                && command
                    .end_time
                    .is_none_or(|end_time| action.start_time <= end_time)
        })
        .copied()
}

fn display_links(
    parent_links: &BTreeMap<String, SemanticActionLink>,
    root_links: &BTreeMap<String, SemanticActionLink>,
    root_action_ids: &[String],
) -> Vec<SemanticActionLink> {
    let mut links = parent_links.values().cloned().collect::<Vec<_>>();
    for action_id in root_action_ids {
        if let Some(link) = root_links.get(action_id) {
            links.push(link.clone());
        }
    }
    links.sort_by(|left, right| {
        (
            left.parent_action_id.as_str(),
            left.child_action_id.as_str(),
            left.role.as_str(),
        )
            .cmp(&(
                right.parent_action_id.as_str(),
                right.child_action_id.as_str(),
                right.role.as_str(),
            ))
    });
    links.dedup_by(|left, right| {
        left.parent_action_id == right.parent_action_id
            && left.child_action_id == right.child_action_id
            && left.role == right.role
    });
    links
}

fn display_role_order(role: SemanticActionLinkRole) -> usize {
    DISPLAY_PARENT_ROLES
        .iter()
        .position(|candidate| *candidate == role)
        .expect("display parent role must be configured")
}

fn invalidated_action(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(ACTION_VALID_ATTR)
        .is_some_and(|value| value == VALID_FALSE)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BulkReadRange {
    trace_id: TraceId,
    process: ProcessIdentity,
    first_event_id: u64,
    last_event_id: u64,
}

fn bulk_read_ranges(actions: &[SemanticAction]) -> Vec<BulkReadRange> {
    actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::FileBulkRead)
        .filter_map(|action| {
            Some(BulkReadRange {
                trace_id: action.trace_id,
                process: action.process.clone(),
                first_event_id: action
                    .attributes
                    .get(attrs::file_bulk_read::FIRST_EVENT_ID)?
                    .parse()
                    .ok()?,
                last_event_id: action
                    .attributes
                    .get(attrs::file_bulk_read::LAST_EVENT_ID)?
                    .parse()
                    .ok()?,
            })
        })
        .collect()
}

fn bulk_read_covered_file_read(action: &SemanticAction, ranges: &[BulkReadRange]) -> bool {
    action.kind == SemanticActionKind::FileRead
        && action.evidence.iter().any(|evidence| {
            evidence.kind == SemanticEvidenceKind::Event
                && ranges.iter().any(|range| {
                    range.trace_id == action.trace_id
                        && range.process == action.process
                        && range.first_event_id <= evidence.id
                        && evidence.id <= range.last_event_id
                })
        })
}

#[cfg(test)]
#[path = "unit_tests/action_tree_projection.rs"]
mod tests;

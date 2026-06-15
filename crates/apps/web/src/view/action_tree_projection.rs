//! Display projection for the web action tree.

use std::collections::{BTreeMap, BTreeSet};

use model_core::{ids::TraceId, process::ProcessIdentity};
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkRole,
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

    fn new(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        let actions = valid_actions(actions);
        let action_by_id = actions
            .iter()
            .map(|action| (action.action_id.clone(), action.clone()))
            .collect::<BTreeMap<_, _>>();
        let links = valid_links(links, &action_by_id);
        let actions = legacy_llm_call::normalize_finalized_http_error_calls(actions, &links);
        let action_by_id = actions
            .iter()
            .map(|action| (action.action_id.clone(), action.clone()))
            .collect::<BTreeMap<_, _>>();
        let links = valid_links(links, &action_by_id);
        let parent_links = selected_parent_links(&links);
        let root_links = selected_root_links(&links);
        let parent_links = remove_cycles(parent_links);
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
    actions
        .into_iter()
        .filter(|action| !invalidated_action(action))
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

#[cfg(test)]
#[path = "unit_tests/action_tree_projection.rs"]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use model_core::process::ProcessMembership;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole,
};

use super::index::LineageIndex;
use super::{
    ATTR_AGENT_ACTION_SEQUENCE, ATTR_LINK_SOURCE, ATTR_LINK_VALID,
    ATTR_PROCESS_PARENT_IDENTITY_STATE, LINK_SOURCE_PROCESS_LINEAGE, LINK_VALID_FALSE,
    PROCESS_PARENT_IDENTITY_STATE_CONFLICT,
};

pub fn derive_lineage_links(
    trace_id: TraceId,
    memberships: &[ProcessMembership],
    actions: &[SemanticAction],
    existing_links: &[SemanticActionLink],
) -> Vec<SemanticActionLink> {
    let index = LineageIndex::new(trace_id, memberships, actions);
    let action_by_id = index.action_by_id();
    let observed_parent_keys = observed_parent_keys(trace_id, existing_links);
    let mut desired = BTreeMap::<LinkKey, SemanticActionLink>::new();

    for action in index.actions() {
        if let Some(link) = derive_command_link(trace_id, &index, &observed_parent_keys, action) {
            desired.insert(LinkKey::from(&link), link);
        }
    }

    let agent_pairs = index
        .actions()
        .iter()
        .filter_map(|action| derive_agent_pair(&index, &observed_parent_keys, action))
        .collect::<Vec<_>>();
    append_agent_links(trace_id, &mut desired, agent_pairs);

    let desired_keys = desired.keys().cloned().collect::<BTreeSet<_>>();
    let mut links = desired.into_values().collect::<Vec<_>>();
    links.extend(invalidate_stale_lineage_links(
        trace_id,
        existing_links,
        &desired_keys,
        &action_by_id,
    ));
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
    links
}

fn derive_command_link(
    trace_id: TraceId,
    index: &LineageIndex<'_>,
    observed_parent_keys: &BTreeSet<ChildRoleKey>,
    action: &SemanticAction,
) -> Option<SemanticActionLink> {
    let role = command_child_role(action)?;
    let child_key = ChildRoleKey::new(action, role);
    if observed_parent_keys.contains(&child_key) || parent_identity_has_conflict(action) {
        return None;
    }
    let start = match action.kind {
        SemanticActionKind::CommandInvocation => index.parent_process(&action.process),
        _ => Some(action.process.clone()),
    }?;
    let parent = index.nearest_command(&start, action.start_time, &action.action_id)?;
    if parent.action_id == action.action_id {
        return None;
    }
    Some(lineage_link(
        trace_id,
        &parent.action_id,
        action,
        role,
        BTreeMap::from([(
            ATTR_LINK_SOURCE.to_string(),
            LINK_SOURCE_PROCESS_LINEAGE.to_string(),
        )]),
    ))
}

fn derive_agent_pair<'a>(
    index: &'a LineageIndex<'a>,
    observed_parent_keys: &BTreeSet<ChildRoleKey>,
    action: &'a SemanticAction,
) -> Option<AgentPair<'a>> {
    if !agent_child_candidate(action) || parent_identity_has_conflict(action) {
        return None;
    }
    let role = SemanticActionLinkRole::AgentPerformedAction;
    let child_key = ChildRoleKey::new(action, role);
    if observed_parent_keys.contains(&child_key) {
        return None;
    }
    let agent_process = match action.kind {
        SemanticActionKind::CommandInvocation => index.parent_process(&action.process),
        _ => Some(action.process.clone()),
    }?;
    let agent = index.agent_for_process(&agent_process, action.start_time)?;
    if agent.process == action.process && action.kind == SemanticActionKind::CommandInvocation {
        return None;
    }
    Some(AgentPair {
        agent,
        child: action,
    })
}

fn append_agent_links(
    trace_id: TraceId,
    desired: &mut BTreeMap<LinkKey, SemanticActionLink>,
    mut pairs: Vec<AgentPair<'_>>,
) {
    pairs.sort_by(|left, right| {
        (
            left.agent.action_id.as_str(),
            left.child.start_time,
            left.child.action_id.as_str(),
        )
            .cmp(&(
                right.agent.action_id.as_str(),
                right.child.start_time,
                right.child.action_id.as_str(),
            ))
    });
    let mut next_sequence = BTreeMap::<String, u64>::new();
    for pair in pairs {
        let sequence = next_sequence
            .entry(pair.agent.action_id.clone())
            .or_default();
        let attributes = BTreeMap::from([
            (
                ATTR_LINK_SOURCE.to_string(),
                LINK_SOURCE_PROCESS_LINEAGE.to_string(),
            ),
            (ATTR_AGENT_ACTION_SEQUENCE.to_string(), sequence.to_string()),
        ]);
        *sequence = sequence.saturating_add(1);
        let link = lineage_link(
            trace_id,
            &pair.agent.action_id,
            pair.child,
            SemanticActionLinkRole::AgentPerformedAction,
            attributes,
        );
        desired.insert(LinkKey::from(&link), link);
    }
}

fn invalidate_stale_lineage_links(
    trace_id: TraceId,
    existing_links: &[SemanticActionLink],
    desired_keys: &BTreeSet<LinkKey>,
    action_by_id: &BTreeMap<String, &SemanticAction>,
) -> Vec<SemanticActionLink> {
    existing_links
        .iter()
        .filter(|link| {
            link.trace_id == trace_id
                && is_lineage_link(link)
                && valid_link(link)
                && derived_role(link.role)
                && !desired_keys.contains(&LinkKey::from(*link))
        })
        .map(|link| {
            let evidence = action_by_id
                .get(&link.child_action_id)
                .map(|action| action.evidence.clone())
                .unwrap_or_default();
            SemanticActionLink {
                trace_id,
                parent_action_id: link.parent_action_id.clone(),
                child_action_id: link.child_action_id.clone(),
                role: link.role,
                confidence: SemanticActionLinkConfidence::Derived,
                evidence,
                attributes: BTreeMap::from([
                    (
                        ATTR_LINK_SOURCE.to_string(),
                        LINK_SOURCE_PROCESS_LINEAGE.to_string(),
                    ),
                    (ATTR_LINK_VALID.to_string(), LINK_VALID_FALSE.to_string()),
                ]),
            }
        })
        .collect()
}

fn lineage_link(
    trace_id: TraceId,
    parent_action_id: &str,
    child: &SemanticAction,
    role: SemanticActionLinkRole,
    attributes: BTreeMap<String, String>,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id,
        parent_action_id: parent_action_id.to_string(),
        child_action_id: child.action_id.clone(),
        role,
        confidence: SemanticActionLinkConfidence::Derived,
        evidence: child.evidence.clone(),
        attributes,
    }
}

fn observed_parent_keys(trace_id: TraceId, links: &[SemanticActionLink]) -> BTreeSet<ChildRoleKey> {
    links
        .iter()
        .filter(|link| {
            link.trace_id == trace_id
                && link.confidence == SemanticActionLinkConfidence::Observed
                && valid_link(link)
        })
        .map(|link| ChildRoleKey {
            child_action_id: link.child_action_id.clone(),
            role: link.role,
        })
        .collect()
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

fn agent_child_candidate(action: &SemanticAction) -> bool {
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

fn is_nested_file_write_event(action: &SemanticAction) -> bool {
    action.kind == SemanticActionKind::FileModify
        && matches!(
            action.attributes.get("file.operation").map(String::as_str),
            Some("write" | "writev")
        )
}

fn parent_identity_has_conflict(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(ATTR_PROCESS_PARENT_IDENTITY_STATE)
        .is_some_and(|state| state == PROCESS_PARENT_IDENTITY_STATE_CONFLICT)
}

fn valid_link(link: &SemanticActionLink) -> bool {
    !link
        .attributes
        .get(ATTR_LINK_VALID)
        .is_some_and(|value| value == LINK_VALID_FALSE)
}

fn is_lineage_link(link: &SemanticActionLink) -> bool {
    link.attributes
        .get(ATTR_LINK_SOURCE)
        .is_some_and(|source| source == LINK_SOURCE_PROCESS_LINEAGE)
}

fn derived_role(role: SemanticActionLinkRole) -> bool {
    matches!(
        role,
        SemanticActionLinkRole::AgentPerformedAction
            | SemanticActionLinkRole::CommandContainsFileAccess
            | SemanticActionLinkRole::CommandContainsProcessForkAttempt
            | SemanticActionLinkRole::CommandContainsCommandInvocation
            | SemanticActionLinkRole::CommandContainsLlmCall
    )
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LinkKey {
    parent_action_id: String,
    child_action_id: String,
    role: SemanticActionLinkRole,
}

impl From<&SemanticActionLink> for LinkKey {
    fn from(link: &SemanticActionLink) -> Self {
        Self {
            parent_action_id: link.parent_action_id.clone(),
            child_action_id: link.child_action_id.clone(),
            role: link.role,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ChildRoleKey {
    child_action_id: String,
    role: SemanticActionLinkRole,
}

impl ChildRoleKey {
    fn new(action: &SemanticAction, role: SemanticActionLinkRole) -> Self {
        Self {
            child_action_id: action.action_id.clone(),
            role,
        }
    }
}

struct AgentPair<'a> {
    agent: &'a SemanticAction,
    child: &'a SemanticAction,
}

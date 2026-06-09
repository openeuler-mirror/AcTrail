use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionLink, SemanticActionLinkConfidence, SemanticActionLinkRole,
    SemanticEvidence,
};

use crate::live::actions::{ATTR_LINK_VALID, LINK_VALID_FALSE};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SemanticActionKey {
    pub(super) trace_id: TraceId,
    pub(super) action_id: String,
}

impl From<&SemanticAction> for SemanticActionKey {
    fn from(action: &SemanticAction) -> Self {
        Self {
            trace_id: action.trace_id,
            action_id: action.action_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ActionLinkKey {
    pub(super) trace_id: TraceId,
    pub(super) parent_action_id: String,
    pub(super) child_action_id: String,
    pub(super) role: SemanticActionLinkRole,
}

pub(super) fn invalidate_child_links(
    emitted_links: &BTreeSet<ActionLinkKey>,
    trace_id: TraceId,
    child_action_id: &str,
    role: SemanticActionLinkRole,
    evidence: &[SemanticEvidence],
) -> Vec<SemanticActionLink> {
    emitted_links
        .iter()
        .filter(|key| {
            key.trace_id == trace_id && key.child_action_id == child_action_id && key.role == role
        })
        .map(|key| SemanticActionLink {
            trace_id,
            parent_action_id: key.parent_action_id.clone(),
            child_action_id: key.child_action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Derived,
            evidence: evidence.to_vec(),
            attributes: BTreeMap::from([(
                ATTR_LINK_VALID.to_string(),
                LINK_VALID_FALSE.to_string(),
            )]),
        })
        .collect()
}

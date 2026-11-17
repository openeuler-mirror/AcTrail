use std::collections::BTreeMap;
use std::mem::size_of;

use semantic_action::{SemanticAction, SemanticActionLink, SemanticEvidence};

pub(super) struct AccountedBytes;

impl AccountedBytes {
    pub(super) fn action(action: &SemanticAction) -> usize {
        size_of::<SemanticAction>()
            .saturating_add(action.action_id.len().saturating_mul(2))
            .saturating_add(action.title.len())
            .saturating_add(Self::attributes(&action.attributes))
            .saturating_add(Self::evidence(&action.evidence))
    }

    pub(super) fn link(link: &SemanticActionLink) -> usize {
        size_of::<SemanticActionLink>()
            .saturating_add(link.parent_action_id.len())
            .saturating_add(link.child_action_id.len())
            .saturating_add(Self::attributes(&link.attributes))
            .saturating_add(Self::evidence(&link.evidence))
    }

    pub(super) fn attribute(key: &str, value: &str) -> usize {
        size_of::<(String, String)>()
            .saturating_add(key.len())
            .saturating_add(value.len())
    }

    pub(super) fn evidence_item(item: &SemanticEvidence) -> usize {
        size_of::<SemanticEvidence>().saturating_add(item.role.len())
    }

    fn attributes(attributes: &BTreeMap<String, String>) -> usize {
        attributes.iter().fold(0usize, |bytes, (key, value)| {
            bytes.saturating_add(Self::attribute(key, value))
        })
    }

    fn evidence(evidence: &[SemanticEvidence]) -> usize {
        evidence.iter().fold(0usize, |bytes, item| {
            bytes.saturating_add(Self::evidence_item(item))
        })
    }
}

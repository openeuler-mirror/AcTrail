//! Private semantic-link construction for tool projection.

use std::collections::BTreeMap;

use semantic_action::{
    LlmRequestLineageWrite, SemanticAction, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole,
};

pub(super) fn lineage_links(lineages: &[LlmRequestLineageWrite]) -> Vec<SemanticActionLink> {
    lineages
        .iter()
        .flat_map(|lineage| {
            let parent = lineage.parent_action_id.as_ref().map(|parent| {
                lineage_link(
                    lineage,
                    parent,
                    SemanticActionLinkRole::LlmRequestTrajectoryParent,
                )
            });
            let fork = lineage.forked_from_action_id.as_ref().map(|parent| {
                lineage_link(
                    lineage,
                    parent,
                    SemanticActionLinkRole::LlmRequestTrajectoryFork,
                )
            });
            parent.into_iter().chain(fork)
        })
        .collect()
}

fn lineage_link(
    lineage: &LlmRequestLineageWrite,
    parent_action_id: &str,
    role: SemanticActionLinkRole,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: lineage.trace_id,
        parent_action_id: parent_action_id.to_string(),
        child_action_id: lineage.action_id.clone(),
        role,
        confidence: SemanticActionLinkConfidence::Derived,
        valid: true,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

pub(super) fn action_link(
    parent: &SemanticAction,
    child: &SemanticAction,
    role: SemanticActionLinkRole,
    confidence: SemanticActionLinkConfidence,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: child.trace_id,
        parent_action_id: parent.action_id.clone(),
        child_action_id: child.action_id.clone(),
        role,
        confidence,
        valid: true,
        evidence: child.evidence.clone(),
        attributes: BTreeMap::new(),
    }
}

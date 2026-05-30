//! Snapshot-time semantic action projection.

mod encoding;
mod http;
mod llm;

use semantic_action::SemanticAction;
use store_snapshot_contract::view::SnapshotView;

pub fn project_snapshot_actions(snapshot: &SnapshotView) -> Vec<SemanticAction> {
    let mut actions = Vec::new();
    actions.extend(llm::project_llm_request_actions(snapshot));
    actions
}

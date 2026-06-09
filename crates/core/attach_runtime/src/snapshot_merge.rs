//! Merge rules between process snapshots and live observations.

use collector_event::RawCollectorEvent;
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership};
use process_tree_snapshot_contract::snapshot::TreeSnapshot;

use crate::identity_merge::merge_identities;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotMergeResult {
    pub memberships: Vec<ProcessMembership>,
    pub bootstrap_partial: bool,
}

pub fn merge_snapshot(
    trace_id: TraceId,
    root_identity: &ProcessIdentity,
    snapshot: &TreeSnapshot,
    live_events: &[RawCollectorEvent],
) -> SnapshotMergeResult {
    let identity_summary = merge_identities(snapshot, live_events);
    let mut memberships = Vec::new();
    let mut bootstrap_partial = !identity_summary.live_only.is_empty();

    for process in &snapshot.processes {
        if process.identity == *root_identity {
            continue;
        }
        let Some(parent) = &process.parent else {
            bootstrap_partial = true;
            continue;
        };
        memberships.push(ProcessMembership::inherited(
            trace_id,
            process.identity.clone(),
            parent.clone(),
            snapshot.captured_at,
        ));
    }

    SnapshotMergeResult {
        memberships,
        bootstrap_partial,
    }
}

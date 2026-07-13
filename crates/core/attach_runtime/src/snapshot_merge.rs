//! Merge rules between process snapshots and live observations.

use collector_event::RawCollectorEvent;
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership, ProcessRecord};
use process_identity::{ProcessIdentityError, ProcessIdentityManager};
use process_tree_snapshot_contract::snapshot::TreeSnapshot;

use crate::identity_merge::merge_identities;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotMergeResult {
    pub memberships: Vec<ProcessMembership>,
    pub bootstrap_partial: bool,
    pub process_records: Vec<ProcessRecord>,
}

pub fn merge_snapshot(
    trace_id: TraceId,
    root_identity: &ProcessIdentity,
    snapshot: &TreeSnapshot,
    live_events: &[RawCollectorEvent],
    process_registry: &mut ProcessIdentityManager,
) -> Result<SnapshotMergeResult, ProcessIdentityError> {
    let identity_summary = merge_identities(snapshot, live_events);
    let mut memberships = Vec::new();
    let mut process_records = std::collections::BTreeMap::new();
    let mut bootstrap_partial = !identity_summary.live_only.is_empty();

    for process in &snapshot.processes {
        let resolution = process_registry.resolve_or_create(process.identity.clone())?;
        let identity = resolution.identity;
        if resolution.created || resolution.enriched {
            process_records.insert(
                identity,
                process_registry
                    .record(identity)
                    .expect("resolved process has a record")
                    .clone(),
            );
        }
        if identity == *root_identity {
            continue;
        }
        let Some(parent_observation) = &process.parent else {
            bootstrap_partial = true;
            continue;
        };
        let parent_resolution = process_registry.resolve_or_create(parent_observation.clone())?;
        let parent = parent_resolution.identity;
        if parent_resolution.created || parent_resolution.enriched {
            process_records.insert(
                parent,
                process_registry
                    .record(parent)
                    .expect("resolved parent has a record")
                    .clone(),
            );
        }
        memberships.push(ProcessMembership::inherited(
            trace_id,
            identity,
            parent,
            snapshot.captured_at,
        ));
    }

    Ok(SnapshotMergeResult {
        memberships,
        bootstrap_partial,
        process_records: process_records.into_values().collect(),
    })
}

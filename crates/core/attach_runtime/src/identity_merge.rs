//! Merge rules for identity-bearing observations during attach.

use std::collections::BTreeSet;

use collector_event::RawCollectorEvent;
use model_core::process::ProcessIdentity;
use process_tree_snapshot_contract::snapshot::TreeSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityMergeSummary {
    pub snapshot_only: BTreeSet<ProcessIdentity>,
    pub live_only: BTreeSet<ProcessIdentity>,
    pub observed_in_both: BTreeSet<ProcessIdentity>,
}

pub fn merge_identities(
    snapshot: &TreeSnapshot,
    live_events: &[RawCollectorEvent],
) -> IdentityMergeSummary {
    let snapshot_identities = snapshot
        .processes
        .iter()
        .map(|process| process.identity.clone())
        .collect::<BTreeSet<_>>();
    let live_identities = live_events
        .iter()
        .map(|event| event.envelope.process.clone())
        .collect::<BTreeSet<_>>();

    let snapshot_only = snapshot_identities
        .difference(&live_identities)
        .cloned()
        .collect();
    let live_only = live_identities
        .difference(&snapshot_identities)
        .cloned()
        .collect();
    let observed_in_both = snapshot_identities
        .intersection(&live_identities)
        .cloned()
        .collect();

    IdentityMergeSummary {
        snapshot_only,
        live_only,
        observed_in_both,
    }
}

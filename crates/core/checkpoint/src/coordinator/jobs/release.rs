//! Releases the snapshot of a deleted checkpoint.

use checkpoint::SnapshotId;

use crate::coordinator::{CoordinatorError, Shared};
use checkpoint_core_contracts::model::{CheckpointId, SnapshotContext};

pub(crate) fn run(
    shared: &Shared,
    context: &SnapshotContext,
    checkpoint_id: CheckpointId,
    snapshot_id: Vec<u8>,
) -> Result<(), CoordinatorError> {
    let snapshot = SnapshotId(snapshot_id.into());
    let failure = shared
        .call(context, |backend| backend.delete(&snapshot))
        .err()
        .map(Shared::backend_failure);
    Ok(shared
        .catalog()
        .record_release(context.trace_id, checkpoint_id, failure.as_ref())?)
}

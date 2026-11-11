//! Disconnect: delete what is left of a Trace's checkpoints and end its backend connection, which
//! frees their snapshots.

use std::time::SystemTime;

use super::records::PendingRecord;
use super::restore;
use crate::coordinator::{CoordinatorError, Hold, Shared};
use crate::model::{CheckpointRecord, Deletion, Disconnected, Failure, Release, SnapshotContext};

/// Runs after the work queued before it, so checkpoints that work created are deleted as well.
/// Disconnecting frees every snapshot, so it also settles releases that failed earlier, including
/// those of a failed disconnect this run retries. Returns the Agents an unsuccessful restore kept
/// waiting, which go on now.
pub(crate) fn run(
    shared: &Shared,
    context: &SnapshotContext,
    ended_at: Option<SystemTime>,
) -> Result<Vec<Hold>, CoordinatorError> {
    shared.persist_pending(context)?;
    // Also handles a failed launch, or a successful disconnect whose catalog write was retried.
    if !shared.connected(context) {
        if let Some(ended_at) = ended_at {
            let mut catalog = shared.catalog();
            if catalog.trace(context.trace_id)?.is_some() {
                catalog.mark_trace_ended(context.trace_id, ended_at)?;
            }
        }
        return restore::resume(shared, context);
    }
    let trace_id = context.trace_id;
    let disconnected_at = SystemTime::now();
    let unreleased = {
        let mut catalog = shared.catalog();
        let unreleased: Vec<CheckpointRecord> = catalog
            .checkpoints(trace_id)?
            .into_iter()
            .filter(|checkpoint| {
                !matches!(
                    checkpoint.deletion,
                    Some(Deletion {
                        release: Release::Released,
                        ..
                    })
                )
            })
            .collect();
        for checkpoint in &unreleased {
            catalog.mark_checkpoint_deleted(trace_id, checkpoint.checkpoint_id, disconnected_at)?;
        }
        unreleased
    };
    let failure = disconnect_backend(shared, context);
    shared.record_result(
        context,
        PendingRecord::Disconnect {
            ended_at,
            disconnected: Disconnected {
                disconnected_at,
                failure,
            },
            checkpoints: unreleased
                .iter()
                .map(|checkpoint| checkpoint.checkpoint_id)
                .collect(),
        },
    )?;
    restore::resume(shared, context)
}

/// Disconnects the backend and records the result on the Trace, for a launch that did not finish.
pub(crate) fn record(shared: &Shared, context: &SnapshotContext) -> Result<(), CoordinatorError> {
    let failure = disconnect_backend(shared, context);
    let ended_at = SystemTime::now();
    shared.record_result(
        context,
        PendingRecord::Disconnect {
            ended_at: Some(ended_at),
            disconnected: Disconnected {
                disconnected_at: ended_at,
                failure,
            },
            checkpoints: Vec::new(),
        },
    )
}

/// A backend that failed to disconnect still holds what it could not free, so it is kept.
fn disconnect_backend(shared: &Shared, context: &SnapshotContext) -> Option<Failure> {
    let mut backend = shared.take_backend(context);
    let error = backend.disconnect().into_result().err()?;
    shared.keep_for_retry(context, backend);
    Some(error.into())
}

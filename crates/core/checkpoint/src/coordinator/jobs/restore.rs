//! Restore: stop the live processes, restore the snapshot and keep the pause on failure.

use std::time::SystemTime;

use checkpoint::SnapshotId;

use super::records::PendingRecord;
use crate::coordinator::scheduler::Held;
use crate::coordinator::{CoordinatorError, Hold, Shared, unknown_checkpoint, unknown_trace};
use crate::model::{
    Boundary, CheckpointRecord, Failure, OperationId, OperationOutcome, SnapshotContext,
    TraceRecord,
};

/// Returns the Agents an earlier restore kept waiting when this one succeeds, and `None` when it
/// does not; the Trace then stays held.
pub(crate) fn run(
    shared: &Shared,
    context: &SnapshotContext,
    operation: OperationId,
    boundary: Option<&Boundary>,
) -> Result<Option<Vec<Hold>>, CoordinatorError> {
    let (trace, checkpoint) = match prepare(shared, context, operation, boundary) {
        Ok(prepared) => prepared,
        Err(error) => {
            // No backend call began. Retain the failed result even if the same catalog failure
            // prevents recording it now, so it will not block every later restore forever.
            let _ = finish(
                shared,
                context,
                operation,
                OperationOutcome::Failed,
                Some(Failure::new(error.code.clone(), error.message.clone())),
                Vec::new(),
            );
            return Err(error);
        }
    };

    let paused = match shared.processes().pause(&trace) {
        Ok(paused) => paused,
        Err(error) => {
            finish(
                shared,
                context,
                operation,
                OperationOutcome::Failed,
                Some(Failure::new("pause_failed", error.message)),
                Vec::new(),
            )?;
            return Ok(None);
        }
    };
    // Catalog writes can fail after the processes stopped. Keep ownership of the pause from
    // here, so every exit remains resumable.
    shared.hold(
        context,
        Held {
            processes: paused,
            boundaries: Vec::new(),
        },
    );

    let target = SnapshotId(checkpoint.snapshot_id.into());
    let (outcome, failure, invalidated) =
        match shared.call(context, |backend| backend.restore(&target)) {
            Ok(invalidated) => (OperationOutcome::Succeeded, None, invalidated),
            Err(failure) => (
                OperationOutcome::Failed,
                Some(Failure::from(failure.error)),
                failure.invalidated,
            ),
        };
    let invalidated: Vec<Vec<u8>> = invalidated.into_iter().map(|id| id.0.to_vec()).collect();
    let succeeded = failure.is_none();
    finish(shared, context, operation, outcome, failure, invalidated)?;
    if !succeeded {
        return Ok(None);
    }
    // A success also releases what an earlier restore held.
    resume(shared, context).map(Some)
}

fn prepare(
    shared: &Shared,
    context: &SnapshotContext,
    operation: OperationId,
    boundary: Option<&Boundary>,
) -> Result<(TraceRecord, CheckpointRecord), CoordinatorError> {
    let mut catalog = shared.catalog();
    let trace = catalog
        .trace(context.trace_id)?
        .ok_or_else(|| unknown_trace(context.trace_id))?;
    let target = catalog
        .operation(operation)?
        .and_then(|record| record.target_checkpoint)
        .ok_or_else(|| {
            CoordinatorError::new("catalog", format!("{operation} has no restore target"))
        })?;
    let checkpoint = catalog
        .checkpoint(context.trace_id, target)?
        .ok_or_else(|| unknown_checkpoint(target))?;
    catalog.start_operation(operation, SystemTime::now(), boundary)?;
    Ok((trace, checkpoint))
}

fn finish(
    shared: &Shared,
    context: &SnapshotContext,
    operation: OperationId,
    outcome: OperationOutcome,
    failure: Option<Failure>,
    invalidated: Vec<Vec<u8>>,
) -> Result<(), CoordinatorError> {
    shared.record_result(
        context,
        PendingRecord::Restore {
            operation,
            finished_at: SystemTime::now(),
            outcome,
            failure,
            invalidated,
        },
    )
}

/// Continues the held processes and returns the Agents kept waiting. Keeps everything held when
/// continuing fails, so a later resume can try again.
pub(crate) fn resume(
    shared: &Shared,
    context: &SnapshotContext,
) -> Result<Vec<Hold>, CoordinatorError> {
    let processes = shared
        .traces()
        .get(context)
        .and_then(|state| state.held.as_ref())
        .map(|held| held.processes.clone())
        .unwrap_or_default();
    // Keep the hold visible until continuing succeeds, including boundaries arriving meanwhile.
    if !processes.is_empty() {
        shared
            .processes()
            .resume(&processes)
            .map_err(|error| CoordinatorError::new("resume_failed", error.message))?;
    }
    Ok(shared.take_held(context).boundaries)
}

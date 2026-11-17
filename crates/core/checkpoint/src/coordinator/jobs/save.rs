//! Save: take a snapshot of the Trace and record it as a checkpoint.

use std::time::SystemTime;

use checkpoint::SnapshotId;

use crate::coordinator::{CoordinatorError, Shared};
use checkpoint_core_contracts::model::{
    Boundary, CheckpointId, NewCheckpoint, NewOperation, OperationId, OperationKind,
    OperationOutcome, Phase, SnapshotContext, Trigger,
};

/// The first boundary connects pending context backends. Later boundaries each get a save;
/// their ticket completes afterwards so the TLS response can be released.
pub(crate) fn run_boundary(
    shared: &Shared,
    context: &SnapshotContext,
    boundary: &Boundary,
) -> Result<(), CoordinatorError> {
    // Context capture activates in the Agent loop after init returns. Release this first
    // boundary so the Agent can publish its context before the next combined save.
    if shared
        .call(context, |backend| backend.connect_context())
        .map_err(|error| CoordinatorError::new("connect_failed", error.message.into_string()))?
    {
        return Ok(());
    }
    let operation = shared.catalog().create_operation(&NewOperation {
        trace_id: context.trace_id,
        kind: OperationKind::Save,
        trigger: Trigger::LlmBoundary,
        phase: Phase::Queued,
        requested_at: boundary.observed_at,
        target_checkpoint: None,
    })?;
    run(shared, context, operation, Some(boundary)).map(drop)
}

pub(crate) fn run(
    shared: &Shared,
    context: &SnapshotContext,
    operation: OperationId,
    boundary: Option<&Boundary>,
) -> Result<Option<CheckpointId>, CoordinatorError> {
    shared
        .catalog()
        .start_operation(operation, SystemTime::now(), boundary)?;
    let snapshot = match shared.call(context, |backend| backend.snapshot()) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            shared.catalog().finish_save(
                operation,
                SystemTime::now(),
                OperationOutcome::Failed,
                Some(&Shared::backend_failure(error)),
                None,
            )?;
            return Ok(None);
        }
    };
    let created = shared.catalog().finish_save(
        operation,
        SystemTime::now(),
        OperationOutcome::Succeeded,
        None,
        Some(&NewCheckpoint {
            snapshot_id: snapshot.0.to_vec(),
        }),
    );
    if created.is_err() {
        // This snapshot belongs to no checkpoint. A failed delete only leaves backend space
        // behind, so its result is not recorded.
        let _ = shared.call(context, |backend| {
            backend.delete(&SnapshotId(snapshot.0.clone()))
        });
    }
    Ok(created?)
}

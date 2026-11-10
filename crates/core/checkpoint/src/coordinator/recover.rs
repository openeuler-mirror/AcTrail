//! Closes what an earlier daemon left behind. Backend instances live in the daemon, so they are
//! gone: unfinished operations cannot go on and the checkpoints of its Traces cannot be restored.

use std::time::SystemTime;

use super::CoordinatorError;
use crate::model::{
    DAEMON_STOPPED, Disconnected, Failure, OperationKind, OperationOutcome, Phase, Release,
};
use crate::ports::CheckpointCatalog;

/// Returns how many operations it finished.
pub(crate) fn close_earlier_daemon(
    catalog: &mut dyn CheckpointCatalog,
    now: SystemTime,
) -> Result<usize, CoordinatorError> {
    let stopped = Failure::new(DAEMON_STOPPED, "the daemon stopped before this finished");
    let unfinished = catalog.unfinished_operations()?;
    for operation in &unfinished {
        // An operation that reached the backend may have changed something.
        let outcome = if operation.phase == Phase::Executing {
            OperationOutcome::Unknown
        } else {
            OperationOutcome::Failed
        };
        match operation.kind {
            OperationKind::Save => {
                catalog.finish_save(operation.operation_id, now, outcome, Some(&stopped), None)?;
            }
            OperationKind::Restore => {
                catalog.finish_restore(operation.operation_id, now, outcome, Some(&stopped), &[])?
            }
        }
    }
    for trace_id in catalog.unsettled_traces()? {
        catalog.mark_trace_ended(trace_id, now)?;
        let unsettled = catalog
            .checkpoints(trace_id)?
            .into_iter()
            .filter(|checkpoint| match &checkpoint.deletion {
                None => true,
                Some(deletion) => deletion.release == Release::Pending,
            });
        for checkpoint in unsettled {
            catalog.mark_checkpoint_deleted(trace_id, checkpoint.checkpoint_id, now)?;
            catalog.record_release(trace_id, checkpoint.checkpoint_id, Some(&stopped))?;
        }
        let open = catalog.trace(trace_id)?.is_some_and(|trace| {
            trace
                .disconnected
                .as_ref()
                .is_none_or(Disconnected::can_retry)
        });
        if open {
            catalog.record_disconnect(
                trace_id,
                &Disconnected {
                    disconnected_at: now,
                    failure: Some(stopped.clone()),
                },
            )?;
        }
    }
    Ok(unfinished.len())
}

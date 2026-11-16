//! Retains restore and disconnect results when their catalog writes fail. Later commands finish
//! recording them before admitting more work, without repeating a completed backend call.

use std::time::SystemTime;

use model_core::ids::TraceId;

use crate::coordinator::{CoordinatorError, Shared};
use crate::model::{
    CheckpointId, Disconnected, Failure, OperationId, OperationOutcome, Phase, SnapshotContext,
};
use crate::ports::CheckpointCatalog;

pub(crate) enum PendingRecord {
    Restore {
        operation: OperationId,
        finished_at: SystemTime,
        outcome: OperationOutcome,
        failure: Option<Failure>,
        invalidated: Vec<Vec<u8>>,
    },
    Disconnect {
        ended_at: Option<SystemTime>,
        disconnected: Disconnected,
        checkpoints: Vec<CheckpointId>,
    },
}

impl PendingRecord {
    fn write(
        &self,
        catalog: &mut dyn CheckpointCatalog,
        trace_id: TraceId,
    ) -> Result<(), CoordinatorError> {
        match self {
            Self::Restore {
                operation,
                finished_at,
                outcome,
                failure,
                invalidated,
            } => {
                // An I/O error can leave the caller unsure whether the transaction committed.
                if catalog
                    .operation(*operation)?
                    .is_some_and(|record| record.phase == Phase::Finished)
                {
                    return Ok(());
                }
                catalog.finish_restore(
                    *operation,
                    *finished_at,
                    *outcome,
                    failure.as_ref(),
                    invalidated,
                )?;
            }
            Self::Disconnect {
                ended_at,
                disconnected,
                checkpoints,
            } => {
                if let Some(ended_at) = ended_at {
                    // Creating the Trace record can itself fail during launch. There is then no
                    // history to close, although its backend still had to be disconnected.
                    if catalog.trace(trace_id)?.is_none() {
                        return Ok(());
                    }
                    catalog.mark_trace_ended(trace_id, *ended_at)?;
                }
                catalog.record_disconnect(trace_id, disconnected)?;
                for checkpoint in checkpoints {
                    catalog.record_release(trace_id, *checkpoint, disconnected.failure.as_ref())?;
                }
            }
        }
        Ok(())
    }
}

impl Shared {
    pub(crate) fn record_result(
        &self,
        context: &SnapshotContext,
        record: PendingRecord,
    ) -> Result<(), CoordinatorError> {
        let mut catalog = self.catalog();
        self.traces()
            .get_mut(context)
            .expect("queued contexts have state")
            .pending_record = Some(record);
        self.persist_record(context, &mut **catalog)
    }

    pub(crate) fn persist_pending(
        &self,
        context: &SnapshotContext,
    ) -> Result<(), CoordinatorError> {
        self.persist_record(context, &mut **self.catalog())
    }

    fn persist_record(
        &self,
        context: &SnapshotContext,
        catalog: &mut dyn CheckpointCatalog,
    ) -> Result<(), CoordinatorError> {
        let record = self
            .traces()
            .get_mut(context)
            .and_then(|state| state.pending_record.take());
        let Some(record) = record else {
            return Ok(());
        };
        if let Err(error) = record.write(catalog, context.trace_id) {
            self.traces()
                .get_mut(context)
                .expect("pending records have state")
                .pending_record = Some(record);
            return Err(error);
        }
        Ok(())
    }
}

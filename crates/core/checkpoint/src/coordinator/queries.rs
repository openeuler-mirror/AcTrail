//! Queries: history through the read-only connection, and live holds and worker activity from
//! memory. They run on the caller's thread without queueing behind checkpoint work or taking
//! the catalog's write lock.

use std::fmt::Display;

use super::{CheckpointCoordinator, CoordinatorError};
use crate::model::{
    CheckpointRecord, ContextStatus, FilterMode, OperationId, OperationRecord, SnapshotContext,
    TraceOwner,
};

impl CheckpointCoordinator {
    /// Current checkpoint-owned holds and worker activity, not a historical restore outcome.
    pub fn status(&self, context: &SnapshotContext) -> ContextStatus {
        let traces = self.shared.traces();
        let state = traces.get(context);
        ContextStatus {
            held: state.is_some_and(|state| state.held.is_some()),
            busy: state.is_some_and(|state| state.busy()),
        }
    }

    pub fn list(
        &self,
        context: &SnapshotContext,
        filter_mode: &FilterMode,
    ) -> Result<Vec<CheckpointRecord>, CoordinatorError> {
        Ok(self.reader.checkpoints(context.trace_id, filter_mode)?)
    }

    pub fn operations(
        &self,
        context: &SnapshotContext,
    ) -> Result<Vec<OperationRecord>, CoordinatorError> {
        Ok(self.reader.operations(context.trace_id)?)
    }

    pub fn operation(
        &self,
        operation_id: OperationId,
    ) -> Result<OperationRecord, CoordinatorError> {
        self.reader
            .operation(operation_id)?
            .ok_or_else(|| not_found(operation_id))
    }

    /// The owner recorded at launch, which stays available after the Trace leaves the runtime.
    pub fn owner(&self, context: &SnapshotContext) -> Result<TraceOwner, CoordinatorError> {
        self.reader
            .trace(context.trace_id)?
            .map(|trace| trace.owner)
            .ok_or_else(|| not_found(context.trace_id))
    }
}

fn not_found(what: impl Display) -> CoordinatorError {
    CoordinatorError::new("not_found", format!("{what} has no checkpoint record"))
}

//! External capabilities the coordinator relies on.

use std::fmt;
use std::time::SystemTime;

use model_core::ids::TraceId;

use crate::model::{
    Boundary, CheckpointId, CheckpointRecord, Disconnected, Failure, FilterMode, NewCheckpoint,
    NewOperation, OperationId, OperationOutcome, OperationRecord, TraceRecord,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogError {
    pub message: String,
}

impl CatalogError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CatalogError {}

/// Reads and writes Trace records, checkpoints and operations. Each method is one short
/// transaction; backend calls and process control happen outside of them.
pub trait CheckpointCatalog {
    fn create_trace(&mut self, trace: &TraceRecord) -> Result<(), CatalogError>;

    fn trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, CatalogError>;

    fn mark_trace_ended(
        &mut self,
        trace_id: TraceId,
        ended_at: SystemTime,
    ) -> Result<(), CatalogError>;

    fn record_disconnect(
        &mut self,
        trace_id: TraceId,
        disconnected: &Disconnected,
    ) -> Result<(), CatalogError>;

    /// Traces an earlier daemon left without an end time, without a successful disconnect,
    /// or with a release that never got an answer.
    fn unsettled_traces(&self) -> Result<Vec<TraceId>, CatalogError>;

    fn create_operation(&mut self, operation: &NewOperation) -> Result<OperationId, CatalogError>;

    /// Moves the operation to `Executing`, recording the boundary it runs at, if any.
    fn start_operation(
        &mut self,
        operation_id: OperationId,
        started_at: SystemTime,
        boundary: Option<&Boundary>,
    ) -> Result<(), CatalogError>;

    /// Finishes a save and creates its checkpoint, if any, in the same transaction.
    fn finish_save(
        &mut self,
        operation_id: OperationId,
        finished_at: SystemTime,
        outcome: OperationOutcome,
        failure: Option<&Failure>,
        checkpoint: Option<&NewCheckpoint>,
    ) -> Result<Option<CheckpointId>, CatalogError>;

    /// Finishes a restore and marks the checkpoints of the invalidated snapshots.
    fn finish_restore(
        &mut self,
        operation_id: OperationId,
        finished_at: SystemTime,
        outcome: OperationOutcome,
        failure: Option<&Failure>,
        invalidated: &[Vec<u8>],
    ) -> Result<(), CatalogError>;

    /// Marks the checkpoint deleted and returns its snapshot ID.
    fn mark_checkpoint_deleted(
        &mut self,
        trace_id: TraceId,
        checkpoint_id: CheckpointId,
        deleted_at: SystemTime,
    ) -> Result<Vec<u8>, CatalogError>;

    /// Records the backend's answer to the deletion; no failure means the snapshot was released.
    fn record_release(
        &mut self,
        trace_id: TraceId,
        checkpoint_id: CheckpointId,
        failure: Option<&Failure>,
    ) -> Result<(), CatalogError>;

    fn checkpoint(
        &self,
        trace_id: TraceId,
        checkpoint_id: CheckpointId,
    ) -> Result<Option<CheckpointRecord>, CatalogError>;

    fn checkpoints(&self, trace_id: TraceId) -> Result<Vec<CheckpointRecord>, CatalogError>;

    fn operation(&self, operation_id: OperationId)
    -> Result<Option<OperationRecord>, CatalogError>;

    fn operations(&self, trace_id: TraceId) -> Result<Vec<OperationRecord>, CatalogError>;

    /// Operations a previous daemon left unfinished.
    fn unfinished_operations(&self) -> Result<Vec<OperationRecord>, CatalogError>;
}

/// Read-only access for queries, on a connection of its own, so a query never waits for the
/// catalog lock that checkpoint work holds while it writes.
pub trait CatalogReader {
    fn trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, CatalogError>;

    fn checkpoints(
        &self,
        trace_id: TraceId,
        filter_mode: &FilterMode,
    ) -> Result<Vec<CheckpointRecord>, CatalogError>;

    fn operations(&self, trace_id: TraceId) -> Result<Vec<OperationRecord>, CatalogError>;

    fn operation(&self, operation_id: OperationId)
    -> Result<Option<OperationRecord>, CatalogError>;
}

/// A process that checkpoint stopped, identified so a reused PID is not continued by mistake.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PausedProcess {
    pub host_pid: u32,
    pub start_time_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessControlError {
    pub message: String,
}

impl ProcessControlError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Stops the processes of a Trace before its files are restored.
pub trait ProcessControl: Send + Sync {
    /// Stops every live process of the Trace, starting from its root, and returns the processes
    /// this call stopped; processes that were already stopped are not returned. When it fails,
    /// none of the processes it stopped stay stopped.
    fn pause(&self, trace: &TraceRecord) -> Result<Vec<PausedProcess>, ProcessControlError>;

    /// Continues the given processes, skipping those that have exited.
    fn resume(&self, processes: &[PausedProcess]) -> Result<(), ProcessControlError>;
}

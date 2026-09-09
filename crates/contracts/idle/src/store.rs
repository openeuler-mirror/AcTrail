//! Storage contracts for idle detection.

use model_core::ids::TraceId;

use crate::{IdleInterval, IdleIntervalId, IdleStoreError};

/// A change emitted by the idle detector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdleStoreOp {
    UpsertInterval(IdleInterval),
    DeleteInterval(IdleIntervalId),
}
/// Write interface for idle state.
pub trait IdleWriteStore {
    /// Applies a batch of detector changes.
    fn apply_idle_ops(&mut self, ops: &[IdleStoreOp]) -> Result<(), IdleStoreError>;
}
/// Read interface for idle state.
pub trait IdleReadStore {
    /// Reads one trace's historical idle intervals.
    fn idle_intervals_for_trace(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<IdleInterval>, IdleStoreError>;
}

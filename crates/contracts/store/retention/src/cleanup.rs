//! Terminal-trace cleanup contracts.

use model_core::ids::TraceId;
use model_core::trace::{TraceHealth, TraceLifecycleState};

use crate::RetentionError;
use crate::tombstone::TraceTombstone;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetentionCandidate {
    pub trace_id: TraceId,
    pub lifecycle_state: TraceLifecycleState,
    pub health: TraceHealth,
}

pub trait RetentionStore {
    fn list_terminal_candidates(&self) -> Result<Vec<RetentionCandidate>, RetentionError>;
    fn purge_trace(
        &mut self,
        trace_id: TraceId,
        tombstone: TraceTombstone,
    ) -> Result<(), RetentionError>;
}

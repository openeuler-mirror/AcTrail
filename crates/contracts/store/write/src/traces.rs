//! Trace-write contracts.

use model_core::ids::TraceId;
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};

use crate::WriteError;

pub trait TraceWriteStore {
    fn create_trace(&mut self, trace: TraceRecord) -> Result<(), WriteError>;
    fn update_trace_lifecycle(
        &mut self,
        trace_id: TraceId,
        lifecycle_state: TraceLifecycleState,
    ) -> Result<(), WriteError>;
    fn update_trace_health(
        &mut self,
        trace_id: TraceId,
        health: TraceHealth,
    ) -> Result<(), WriteError>;
}

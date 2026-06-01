//! Tombstone contracts retained after trace purge.

use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::trace::{TraceHealth, TraceLifecycleState};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceTombstone {
    pub trace_id: TraceId,
    pub lifecycle_state: TraceLifecycleState,
    pub health: TraceHealth,
    pub cleaned_at: SystemTime,
    pub cleanup_reason: String,
}

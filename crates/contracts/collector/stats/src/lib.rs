//! Collector health and drop-counter contracts.

use std::time::SystemTime;

use model_core::ids::CollectorName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DropCounter {
    pub reason: String,
    pub count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectorStats {
    pub collector_name: CollectorName,
    pub active_bindings: usize,
    pub last_heartbeat_at: SystemTime,
    pub dropped: Vec<DropCounter>,
}

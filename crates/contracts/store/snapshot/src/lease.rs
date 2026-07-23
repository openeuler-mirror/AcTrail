//! Tokenized trace leases shared by snapshot export and post-trace analysis.

use std::fmt;
use std::time::SystemTime;

use model_core::ids::TraceId;

use crate::SnapshotError;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraceLeaseToken(u64);

impl TraceLeaseToken {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TraceLeaseToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "trace-lease-{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceLeasePurpose {
    Export,
    PostTraceAnalysis,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceLease {
    pub token: TraceLeaseToken,
    pub trace_id: TraceId,
    pub purpose: TraceLeasePurpose,
    pub granted_at: SystemTime,
}

pub trait SnapshotLeaseStore {
    fn acquire_trace_lease(
        &mut self,
        trace_id: TraceId,
        purpose: TraceLeasePurpose,
    ) -> Result<TraceLease, SnapshotError>;

    fn release_trace_lease(&mut self, lease: TraceLease) -> Result<(), SnapshotError>;
}

//! Export-lease contracts used to serialize export and retention.

use std::time::SystemTime;

use model_core::ids::TraceId;

use crate::SnapshotError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportLease {
    pub trace_id: TraceId,
    pub granted_at: SystemTime,
}

pub trait SnapshotLeaseStore {
    fn acquire_export_lease(&mut self, trace_id: TraceId) -> Result<ExportLease, SnapshotError>;
    fn release_export_lease(&mut self, lease: ExportLease) -> Result<(), SnapshotError>;
}

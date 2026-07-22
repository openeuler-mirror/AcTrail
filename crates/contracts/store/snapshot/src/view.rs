//! Consistent-view contracts for export snapshot reads.

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::TraceRecord;

use crate::SnapshotError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotView {
    pub trace: TraceRecord,
    pub memberships: Vec<ProcessMembership>,
    pub events: Vec<DomainEvent>,
    pub payload_segments: Vec<PayloadSegment>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

pub trait SnapshotStore {
    fn read_snapshot(
        &self,
        lease: &crate::lease::TraceLease,
    ) -> Result<SnapshotView, SnapshotError>;
}

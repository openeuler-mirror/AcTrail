//! Track-add, track-remove, and lifecycle command ownership.

use std::collections::BTreeSet;
use std::time::SystemTime;

use config_core::trace_snapshot::CaptureProfileSnapshot;
use model_core::ids::{TraceId, TraceName};
use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackTraceRequest {
    pub root_identity: ProcessIdentity,
    pub display_name: TraceName,
    pub profile_snapshot: CaptureProfileSnapshot,
    pub tags: BTreeSet<String>,
    pub created_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootRemovalRequest {
    pub trace_id: TraceId,
    pub removed_at: SystemTime,
}

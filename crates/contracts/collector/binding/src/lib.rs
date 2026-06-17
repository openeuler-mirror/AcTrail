//! Trace-to-collector binding contracts.

use std::time::SystemTime;

use collector_capability::CollectorDescriptor;
use config_core::trace_snapshot::CaptureProfileSnapshot;
use model_core::capability::CapabilityRequest;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{InitialSuppressedFd, ProcessIdentity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageGuardRequest {
    pub trace_id: TraceId,
    pub root_identity: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageGuardHandle {
    pub collector_name: CollectorName,
    pub activated_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceBindingRequest {
    pub trace_id: TraceId,
    pub root_identity: ProcessIdentity,
    pub profile_snapshot: CaptureProfileSnapshot,
    pub requested_capabilities: Vec<CapabilityRequest>,
    pub initial_suppressed_fds: Vec<InitialSuppressedFd>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceBindingHandle {
    pub collector: CollectorDescriptor,
    pub bound_at: SystemTime,
}

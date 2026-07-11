//! Logical process identity, OS coordinates, and trace membership semantics.

use std::time::SystemTime;

pub use process_identity::{
    HostProcessCoordinates, InitialSuppressedFd, KernelProcessCoordinates, NamespaceIdentity,
    NamespaceProcessCoordinates, ProcessIdentity, ProcessObservation, ProcessRecord,
    ProcessResolutionState, ProcessSuppressedFd, SuppressedFdPurpose,
};

use crate::ids::TraceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipState {
    Starting,
    Active,
    Exited,
    IdentityStale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub observed_at: SystemTime,
    pub source: Option<ExitObservationSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitObservationSource {
    Event,
    Reconciled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMembership {
    pub trace_id: TraceId,
    pub identity: ProcessIdentity,
    pub inherited_from: Option<ProcessIdentity>,
    pub observed_at: Option<SystemTime>,
    pub capture_enabled: bool,
    pub propagation_enabled: bool,
    pub state: MembershipState,
    pub exit_status: Option<ExitStatus>,
}

impl ProcessMembership {
    pub fn root(trace_id: TraceId, identity: ProcessIdentity, observed_at: SystemTime) -> Self {
        Self {
            trace_id,
            identity,
            inherited_from: None,
            observed_at: Some(observed_at),
            capture_enabled: true,
            propagation_enabled: true,
            state: MembershipState::Starting,
            exit_status: None,
        }
    }

    pub fn inherited(
        trace_id: TraceId,
        identity: ProcessIdentity,
        inherited_from: ProcessIdentity,
        observed_at: SystemTime,
    ) -> Self {
        Self {
            trace_id,
            identity,
            inherited_from: Some(inherited_from),
            observed_at: Some(observed_at),
            capture_enabled: true,
            propagation_enabled: true,
            state: MembershipState::Starting,
            exit_status: None,
        }
    }

    pub fn observed(trace_id: TraceId, identity: ProcessIdentity, observed_at: SystemTime) -> Self {
        Self {
            trace_id,
            identity,
            inherited_from: None,
            observed_at: Some(observed_at),
            capture_enabled: true,
            propagation_enabled: true,
            state: MembershipState::Starting,
            exit_status: None,
        }
    }

    pub fn activate(&mut self) {
        self.state = MembershipState::Active;
    }

    pub fn disable_capture(&mut self) {
        self.capture_enabled = false;
    }

    pub fn disable_propagation(&mut self) {
        self.propagation_enabled = false;
    }

    pub fn can_inherit(&self) -> bool {
        self.capture_enabled
            && self.propagation_enabled
            && matches!(
                self.state,
                MembershipState::Starting | MembershipState::Active
            )
    }

    pub fn mark_exited(&mut self, status: ExitStatus) {
        self.state = MembershipState::Exited;
        self.exit_status = Some(status);
    }

    pub fn mark_identity_stale(&mut self) {
        self.state = MembershipState::IdentityStale;
    }
}

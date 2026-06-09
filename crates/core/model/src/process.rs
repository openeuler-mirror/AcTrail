//! Process identity and membership semantics.

use std::time::SystemTime;

use crate::ids::TraceId;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NamespaceIdentity(String);

impl NamespaceIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub task_id: Option<u32>,
    pub start_time_ticks: u64,
    pub pid_namespace: Option<NamespaceIdentity>,
    pub generation: u64,
}

impl ProcessIdentity {
    pub fn new(pid: u32, start_time_ticks: u64, generation: u64) -> Self {
        Self {
            pid,
            task_id: None,
            start_time_ticks,
            pid_namespace: None,
            generation,
        }
    }

    pub fn with_task_id(mut self, task_id: u32) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn with_namespace(mut self, pid_namespace: NamespaceIdentity) -> Self {
        self.pid_namespace = Some(pid_namespace);
        self
    }
}

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

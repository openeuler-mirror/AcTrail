//! Records kept in the checkpoint catalog.

use std::fmt;
use std::time::SystemTime;

use checkpoint::{BackendError, BackendErrorKind};
use model_core::ids::TraceId;
use serde::{Deserialize, Serialize};

/// Numbered from 0 within its Trace, where 0 is the initial checkpoint taken at launch; a
/// checkpoint is named by its Trace and this id.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CheckpointId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId(u64);

impl CheckpointId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl OperationId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "checkpoint-{}", self.0)
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "operation-{}", self.0)
    }
}

/// A scope as written in the launch configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScopeConfig {
    pub id: String,
    pub root: String,
    pub excludes: Vec<String>,
    pub backend_instance: String,
}

/// Copied from the Trace owner so commands stay authorized after the Trace leaves the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceOwner {
    pub uid: u32,
    pub pid_namespace: String,
    pub mount_namespace: String,
    pub host_pid_namespace: bool,
    pub host_mount_namespace: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootProcess {
    pub host_pid: u32,
    pub start_time_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRecord {
    pub trace_id: TraceId,
    pub owner: TraceOwner,
    pub root: RootProcess,
    pub scopes: Vec<ScopeConfig>,
    pub created_at: SystemTime,
    /// Set when the Trace ends, launch fails, or its daemon run is closed.
    pub ended_at: Option<SystemTime>,
    /// Set once the backend was disconnected; its snapshots are gone.
    pub disconnected: Option<Disconnected>,
}

/// The live state owned by this daemon, queried independently of checkpoint history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextStatus {
    /// The coordinator still holds processes or LLM response boundaries after a restore.
    pub held: bool,
    /// A worker is running or has queued work for this context.
    pub busy: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Disconnected {
    pub disconnected_at: SystemTime,
    /// Set when the backend could not free everything, which leaves backend space behind.
    pub failure: Option<Failure>,
}

/// The failure code for what an earlier daemon left behind, which no instance can retry.
pub const DAEMON_STOPPED: &str = "daemon_stopped";

impl Disconnected {
    /// A failed disconnect can run again while this daemon keeps the backend.
    pub fn can_retry(&self) -> bool {
        self.failure
            .as_ref()
            .is_some_and(|failure| failure.code != DAEMON_STOPPED)
    }
}

/// Why a backend call or the work around it failed, as kept in the records.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

impl Failure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<BackendError> for Failure {
    fn from(error: BackendError) -> Self {
        let code = match error.kind {
            BackendErrorKind::InvalidConfiguration => "invalid_configuration",
            BackendErrorKind::InvalidSnapshot => "invalid_snapshot",
            BackendErrorKind::Unavailable => "unavailable",
            BackendErrorKind::OperationFailed => "operation_failed",
        };
        Self::new(code, error.message.into_string())
    }
}

macro_rules! storage_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

storage_enum!(OperationKind {
    Save => "save",
    Restore => "restore",
});

storage_enum!(Trigger {
    Launch => "launch",
    LlmBoundary => "llm_boundary",
    Manual => "manual",
});

storage_enum!(Phase {
    Queued => "queued",
    WaitingBoundary => "waiting_boundary",
    Executing => "executing",
    Finished => "finished",
});

storage_enum!(OperationOutcome {
    Succeeded => "succeeded",
    Failed => "failed",
    Unknown => "unknown",
});

/// The LLM request boundary at which an operation ran.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Boundary {
    pub observed_at: SystemTime,
    /// The `llm.request` action whose response waited, which links the operation to its LLM
    /// call in the ActionTree.
    pub request_action_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewOperation {
    pub trace_id: TraceId,
    pub kind: OperationKind,
    pub trigger: Trigger,
    pub phase: Phase,
    pub requested_at: SystemTime,
    pub target_checkpoint: Option<CheckpointId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationRecord {
    pub operation_id: OperationId,
    pub trace_id: TraceId,
    pub kind: OperationKind,
    pub trigger: Trigger,
    pub phase: Phase,
    /// Set when the phase is `Finished`.
    pub outcome: Option<OperationOutcome>,
    pub requested_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    pub target_checkpoint: Option<CheckpointId>,
    pub created_checkpoint: Option<CheckpointId>,
    pub boundary: Option<Boundary>,
    /// Set when the operation did not succeed.
    pub failure: Option<Failure>,
    /// Checkpoints this restore invalidated.
    pub invalidated_checkpoints: Vec<CheckpointId>,
}

/// What a fully successful save adds to the catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewCheckpoint {
    /// The backend's ID, meaningful only to the instance that issued it.
    pub snapshot_id: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Release {
    /// The backend has not been asked yet.
    Pending,
    Released,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deletion {
    pub deleted_at: SystemTime,
    pub release: Release,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointRecord {
    pub checkpoint_id: CheckpointId,
    pub trace_id: TraceId,
    pub operation_id: OperationId,
    pub created_at: SystemTime,
    pub snapshot_id: Vec<u8>,
    /// The restore that invalidated its snapshot.
    pub invalidated_by: Option<OperationId>,
    pub deletion: Option<Deletion>,
}

impl CheckpointRecord {
    pub fn restorable(&self) -> bool {
        self.invalidated_by.is_none() && self.deletion.is_none()
    }
}

/// Whose checkpoints a call is about. Today that is one Trace; a narrower subject, such as one of
/// several sessions in a Trace, adds a field here instead of changing the interfaces that take it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SnapshotContext {
    pub trace_id: TraceId,
}

/// How a list selects checkpoints. Every mode returns them in creation order, deleted ones
/// included.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterMode {
    All,
    /// The newest this many checkpoints.
    Last(u32),
    /// Checkpoints created in `[since, until)`; a missing bound leaves that end open.
    Created {
        since: Option<SystemTime>,
        until: Option<SystemTime>,
    },
}

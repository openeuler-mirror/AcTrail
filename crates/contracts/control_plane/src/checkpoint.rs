//! Checkpoint commands, replies and the launch configuration carried by `TrackAdd`.

use model_core::ids::{RequestId, TraceId};
use serde::{Deserialize, Serialize};

/// Scopes protected for one launch, read by ctl from the `--checkpoint` file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchCheckpointConfig {
    pub scopes: Vec<LaunchCheckpointScope>,
}

/// Paths are absolute in the Agent's view; relative excludes are resolved against `root`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchCheckpointScope {
    pub id: String,
    pub root: String,
    pub excludes: Vec<String>,
    pub backend_instance: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointCommand {
    pub request_id: RequestId,
    pub action: CheckpointAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointAction {
    /// `CheckpointSave`: save every scope of a running Trace now.
    Save { trace_id: TraceId },
    /// `CheckpointRestore`: restore a Trace to one of its checkpoints. A running Trace is
    /// restored at its next LLM request.
    Restore {
        trace_id: TraceId,
        checkpoint_id: u64,
    },
    /// `CheckpointDelete`: mark a checkpoint deleted and release its snapshot.
    Delete {
        trace_id: TraceId,
        checkpoint_id: u64,
    },
    /// `CheckpointDisconnect`: give up the checkpoints of a Trace. Its remaining checkpoints are
    /// deleted, it takes no more saves or restores, and the backends free what they keep for it.
    Disconnect { trace_id: TraceId },
    /// `CheckpointForce`: start a restore that waits for an LLM request now.
    Force { operation_id: u64 },
    /// `CheckpointResume`: continue the processes a failed restore left stopped.
    Resume { trace_id: TraceId },
    /// `CheckpointList`: the checkpoints of a Trace that `filter_mode` selects.
    List {
        trace_id: TraceId,
        filter_mode: CheckpointFilterMode,
    },
    /// `CheckpointOperations`: the saves and restores of a Trace.
    Operations { trace_id: TraceId },
    /// `CheckpointOperation`: progress and result of one operation.
    Operation { operation_id: u64 },
    /// `CheckpointStatus`: the daemon's current holds and worker activity for a Trace.
    Status { trace_id: TraceId },
}

/// How `CheckpointList` selects checkpoints. Every mode returns them in creation order, deleted
/// ones included.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointFilterMode {
    All,
    /// The newest this many checkpoints.
    Last(u32),
    /// Checkpoints created in `[since, until)`; a missing bound leaves that end open.
    Created {
        since_unix_nanos: Option<u64>,
        until_unix_nanos: Option<u64>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointReply {
    /// A save or restore was recorded and queued.
    OperationAccepted {
        operation_id: u64,
    },
    /// A delete, disconnect, force or resume was accepted.
    Accepted,
    Checkpoints(Vec<CheckpointView>),
    Operations(Vec<OperationView>),
    Operation(Box<OperationView>),
    Status {
        held: bool,
        busy: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckpointView {
    pub checkpoint_id: u64,
    pub operation_id: u64,
    pub created_at_unix_nanos: u64,
    pub restorable: bool,
    pub invalidated_by: Option<u64>,
    pub deleted_at_unix_nanos: Option<u64>,
    /// `pending`, `released` or `failed` once the checkpoint is deleted.
    pub release: Option<String>,
    pub release_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationView {
    pub operation_id: u64,
    pub trace_id: u64,
    /// `save` or `restore`.
    pub kind: String,
    /// `launch`, `llm_boundary` or `manual`.
    pub trigger: String,
    /// `queued`, `waiting_boundary`, `executing` or `finished`.
    pub phase: String,
    /// `succeeded`, `failed` or `unknown` once finished.
    pub outcome: Option<String>,
    pub requested_at_unix_nanos: u64,
    pub started_at_unix_nanos: Option<u64>,
    pub finished_at_unix_nanos: Option<u64>,
    pub target_checkpoint: Option<u64>,
    pub created_checkpoint: Option<u64>,
    /// Why it did not succeed.
    pub error: Option<String>,
    pub invalidated_checkpoints: Vec<u64>,
}

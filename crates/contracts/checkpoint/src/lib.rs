//! Synchronous ABI contracts for leaf and composite checkpoint backends.
//!
//! Connect and disconnect manage checkpoint integration. Delete removes a single
//! snapshot. Each backend documents its capture boundary, restore scope, and
//! whether a successful request reports acceptance or completed execution.

mod backend;
mod error;
mod types;

pub use backend::{
    CheckpointBackend, CheckpointModule, CheckpointModuleRef, RestorableCheckpointBackend,
    RestorableCheckpointBackend_TO,
};
pub use error::{BackendError, BackendErrorKind, RestoreFailure};
pub use types::{
    BackendConfig, BackendKind, SnapshotContext, SnapshotId, SnapshotScope, TraceProcess,
};

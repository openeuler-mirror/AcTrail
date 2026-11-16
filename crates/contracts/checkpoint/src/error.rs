//! Defines ABI-safe operation failures returned by checkpoint backends.

use abi_stable::{
    StableAbi,
    std_types::{RString, RVec},
};

use crate::SnapshotId;

/// Classification of a backend failure; it does not imply retry safety.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
pub enum BackendErrorKind {
    InvalidConfiguration,
    InvalidSnapshot,
    Unavailable,
    OperationFailed,
}

/// Backend failure returned to its caller.
/// Failure may follow partial changes or a lost RPC reply after remote execution.
/// It does not imply that retrying the operation is safe.
#[repr(C)]
#[derive(Clone, Debug, StableAbi)]
pub struct BackendError {
    pub kind: BackendErrorKind,
    pub message: RString,
}

impl BackendError {
    pub fn new(kind: BackendErrorKind, message: impl Into<RString>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// A restore that did not finish, and what it already cost.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct RestoreFailure {
    pub error: BackendError,
    /// Snapshots invalidated before it failed.
    pub invalidated: RVec<SnapshotId>,
}

/// A failure that invalidated nothing.
impl From<BackendError> for RestoreFailure {
    fn from(error: BackendError) -> Self {
        Self {
            error,
            invalidated: RVec::new(),
        }
    }
}

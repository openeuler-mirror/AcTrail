//! Defines ABI-safe operation failures returned by checkpoint backends.

use abi_stable::{StableAbi, std_types::RString};

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
#[derive(Debug, StableAbi)]
pub struct BackendError {
    pub kind: BackendErrorKind,
    pub message: RString,
}

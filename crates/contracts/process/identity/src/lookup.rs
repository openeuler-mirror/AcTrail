//! Identity lookup contracts for root PID attach and runtime verification.

use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityLookupError {
    NotFound { pid: u32 },
    PermissionDenied { pid: u32 },
    Incomplete { pid: u32, detail: String },
}

pub trait ProcessIdentityReader {
    fn read_identity(&self, pid: u32) -> Result<ProcessIdentity, IdentityLookupError>;
}

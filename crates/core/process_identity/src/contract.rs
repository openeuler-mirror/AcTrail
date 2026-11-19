//! Process identity observation interface and lookup errors.

use process_identity_contracts::ProcessObservation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityLookupError {
    NotFound { pid: u32 },
    PermissionDenied { pid: u32 },
    Incomplete { pid: u32, detail: String },
}

pub trait ProcessIdentityReader {
    fn read_identity(&self, pid: u32) -> Result<ProcessObservation, IdentityLookupError>;
}

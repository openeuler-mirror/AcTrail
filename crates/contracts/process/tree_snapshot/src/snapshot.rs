//! Process-tree snapshot contracts for attach bootstrap.

use std::time::SystemTime;

use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSnapshot {
    pub identity: ProcessIdentity,
    pub parent: Option<ProcessIdentity>,
    pub executable: Option<String>,
    pub current_working_directory: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeSnapshot {
    pub root: ProcessIdentity,
    pub captured_at: SystemTime,
    pub processes: Vec<ProcessSnapshot>,
}

pub trait ProcessTreeSnapshotter {
    type Error;

    fn snapshot(&self, root: &ProcessIdentity) -> Result<TreeSnapshot, Self::Error>;
}

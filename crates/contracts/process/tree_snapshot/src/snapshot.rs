//! Process-tree snapshot contracts for attach bootstrap.

use std::time::SystemTime;

use model_core::process::ProcessObservation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSnapshot {
    pub identity: ProcessObservation,
    pub parent: Option<ProcessObservation>,
    pub executable: Option<String>,
    pub current_working_directory: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeSnapshot {
    pub root: ProcessObservation,
    pub captured_at: SystemTime,
    pub processes: Vec<ProcessSnapshot>,
}

pub trait ProcessTreeSnapshotter {
    type Error;

    fn snapshot(&self, root: &ProcessObservation) -> Result<TreeSnapshot, Self::Error>;
}

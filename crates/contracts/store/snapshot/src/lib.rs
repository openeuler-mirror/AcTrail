//! Consistent-read snapshot contracts.

pub mod lease;
pub mod view;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotError {
    pub stage: String,
    pub message: String,
}

impl SnapshotError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

//! Retention and tombstone contracts for terminal traces.

pub mod cleanup;
pub mod tombstone;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetentionError {
    pub stage: String,
    pub message: String,
}

impl RetentionError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

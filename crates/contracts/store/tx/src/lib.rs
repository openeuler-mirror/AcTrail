//! Transaction-boundary contracts for storage implementations.

pub mod boundary;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageTransactionError {
    pub stage: String,
    pub message: String,
}

impl StorageTransactionError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

//! Errors returned by idle storage operations.
use std::fmt;

/// Error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleStoreErrorKind {
    InvalidInterval, // Invalid interval or task snapshot.
    NotFound,        // Requested data is absent.
    StorageFailure,  // Underlying SQLite operation failed.
}

/// Error details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdleStoreError {
    pub kind: IdleStoreErrorKind,
    pub stage: String,
    pub message: String,
}

impl IdleStoreError {
    pub fn new(
        kind: IdleStoreErrorKind,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            stage: stage.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for IdleStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for IdleStoreError {}

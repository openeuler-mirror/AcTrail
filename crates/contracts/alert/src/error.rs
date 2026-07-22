//! Typed failures exposed by the alert storage Interface.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertStoreErrorKind {
    InvalidDefinition,
    DefinitionConflict,
    InvalidPayload,
    NotFound,
    StorageFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertStoreError {
    pub kind: AlertStoreErrorKind,
    pub stage: String,
    pub message: String,
}

impl AlertStoreError {
    pub fn new(
        kind: AlertStoreErrorKind,
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

impl fmt::Display for AlertStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for AlertStoreError {}

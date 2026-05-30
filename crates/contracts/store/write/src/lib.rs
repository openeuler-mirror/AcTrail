//! Write-side persistence contracts.

pub mod diagnostics;
pub mod events;
pub mod memberships;
pub mod payloads;
pub mod traces;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteError {
    pub stage: String,
    pub message: String,
}

impl WriteError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

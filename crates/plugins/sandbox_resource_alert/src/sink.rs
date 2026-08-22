use std::fmt;

use crate::SandboxAlert;

pub trait SandboxAlertSink: Send + Sync + 'static {
    fn try_submit(&self, alert: SandboxAlert) -> Result<(), SandboxAlertSinkError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxAlertSinkError {
    code: String,
    message: String,
}

impl SandboxAlertSinkError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for SandboxAlertSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxAlertSinkError {}

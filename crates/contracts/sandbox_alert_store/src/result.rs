use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAlertSourceError {
    ZeroGatewayId,
    ZeroSbId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAlertAdmission {
    Accepted,
    Full,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxAlertReadError {
    pub code: String,
    pub message: String,
}

impl SandboxAlertReadError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SandboxAlertReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxAlertReadError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxAlertShutdownError {
    pub code: String,
    pub message: String,
}

impl SandboxAlertShutdownError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SandboxAlertShutdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxAlertShutdownError {}

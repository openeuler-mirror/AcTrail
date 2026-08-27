use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerStartError {
    stage: &'static str,
    message: String,
}

impl ServerStartError {
    pub(crate) fn config(message: impl Into<String>) -> Self {
        Self::new("config", message)
    }

    pub(crate) fn new(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }

    pub fn stage(&self) -> &str {
        self.stage
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ServerStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for ServerStartError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerShutdownError {
    message: String,
}

impl ServerShutdownError {
    pub(crate) fn accept_thread_panicked() -> Self {
        Self {
            message: "upstream accept thread panicked".to_string(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ServerShutdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ServerShutdownError {}

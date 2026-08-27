use std::fmt;

/// A bounded startup or sampling failure.
#[derive(Debug)]
pub struct SandboxLinuxError {
    stage: &'static str,
    detail: String,
}

impl SandboxLinuxError {
    pub(crate) fn new(stage: &'static str, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }

    pub fn stage(&self) -> &'static str {
        self.stage
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for SandboxLinuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.detail)
    }
}

impl std::error::Error for SandboxLinuxError {}

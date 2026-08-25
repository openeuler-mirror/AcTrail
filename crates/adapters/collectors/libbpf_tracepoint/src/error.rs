use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TracepointAttachError {
    stage: &'static str,
    detail: String,
}

impl TracepointAttachError {
    pub(crate) fn new(stage: &'static str, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }

    pub const fn stage(&self) -> &'static str {
        self.stage
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for TracepointAttachError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.detail)
    }
}

impl std::error::Error for TracepointAttachError {}

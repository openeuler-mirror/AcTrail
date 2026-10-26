use std::fmt;

#[derive(Debug)]
pub struct ProfilingError {
    pub code: String,
    pub message: String,
    unavailable: bool,
}

impl ProfilingError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            unavailable: false,
        }
    }

    pub(crate) fn availability(mut self, unavailable: bool) -> Self {
        self.unavailable = unavailable;
        self
    }

    pub(crate) fn is_unavailable(&self) -> bool {
        self.unavailable
    }
}

impl fmt::Display for ProfilingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ProfilingError {}

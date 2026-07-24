use std::error::Error;
use std::fmt::{Display, Formatter};

pub(crate) trait ProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectorConfigError {
    message: String,
}

impl DetectorConfigError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for DetectorConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DetectorConfigError {}

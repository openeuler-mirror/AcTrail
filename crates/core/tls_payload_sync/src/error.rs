//! Error boundary for shared sync TLS helpers.

use std::error::Error;
use std::fmt::{Display, Formatter};

pub type SyncResult<T> = Result<T, SyncError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncError {
    message: String,
}

impl SyncError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for SyncError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for SyncError {}

impl From<std::io::Error> for SyncError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<tls_probe_point_finder::ToolError> for SyncError {
    fn from(error: tls_probe_point_finder::ToolError) -> Self {
        Self::new(error.to_string())
    }
}

//! Synchronous TLS payload rewrite probe.

use std::error::Error;
use std::fmt::{Display, Formatter};

mod cli;
mod runtime;

pub fn main_from_env() -> i32 {
    cli::main_from_env()
}

type ToolResult<T> = Result<T, ToolError>;

#[derive(Debug)]
struct ToolError {
    message: String,
}

impl ToolError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for ToolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ToolError {}

impl From<std::io::Error> for ToolError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<tls_payload_core::CoreError> for ToolError {
    fn from(error: tls_payload_core::CoreError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<tls_payload_sync::SyncError> for ToolError {
    fn from(error: tls_payload_sync::SyncError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<tls_probe_point_finder::ToolError> for ToolError {
    fn from(error: tls_probe_point_finder::ToolError) -> Self {
        Self::new(error.to_string())
    }
}

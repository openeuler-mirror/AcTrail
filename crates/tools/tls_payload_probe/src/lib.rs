//! Standalone TLS payload uprobe tool.

use std::error::Error;
use std::fmt::{Display, Formatter};

mod capture;
mod cli;
mod llm_projection;

pub fn run_from_env() -> ToolResult<()> {
    cli::run_from_env()
}

pub fn main_from_env() -> i32 {
    cli::main_from_env()
}

pub type ToolResult<T> = Result<T, ToolError>;

#[derive(Debug)]
pub struct ToolError {
    message: String,
}

impl ToolError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
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

impl From<tls_probe_point_finder::ToolError> for ToolError {
    fn from(error: tls_probe_point_finder::ToolError) -> Self {
        Self::new(error.to_string())
    }
}

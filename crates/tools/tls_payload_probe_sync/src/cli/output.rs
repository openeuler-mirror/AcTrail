//! CLI output sinks.

use std::fmt::Display;
use std::io::{self, Write};

use crate::ToolResult;

pub(crate) struct Output;

impl Output {
    pub(crate) fn stdout(text: &str) -> ToolResult<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(text.as_bytes())?;
        stdout.flush()?;
        Ok(())
    }

    pub(crate) fn stderr(text: &str) -> ToolResult<()> {
        let mut stderr = io::stderr().lock();
        stderr.write_all(text.as_bytes())?;
        stderr.flush()?;
        Ok(())
    }
}

pub(crate) fn write_error(error: &dyn Display) -> ToolResult<()> {
    Output::stderr(&format!("error: {error}\n"))
}

//! Transport-local bind, connection, framing, and command I/O failures.

use std::error::Error;
use std::fmt;

const MAX_ERROR_MESSAGE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxControlUdsStage {
    Configure,
    Bind,
    Accept,
    Connect,
    Read,
    Decode,
    Dispatch,
    Encode,
    Write,
    Join,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxControlUdsError {
    stage: SandboxControlUdsStage,
    message: String,
}

impl SandboxControlUdsError {
    pub fn new(stage: SandboxControlUdsStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: bounded_message(message.into()),
        }
    }

    pub const fn stage(&self) -> SandboxControlUdsStage {
        self.stage
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for SandboxControlUdsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "sandbox control {:?}: {}",
            self.stage, self.message
        )
    }
}

impl Error for SandboxControlUdsError {}

fn bounded_message(mut message: String) -> String {
    if message.len() <= MAX_ERROR_MESSAGE_BYTES {
        return message;
    }
    let mut end = MAX_ERROR_MESSAGE_BYTES - 3;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.push_str("...");
    message
}

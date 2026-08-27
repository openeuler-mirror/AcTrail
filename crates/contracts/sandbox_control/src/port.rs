//! Runtime port implemented by the sandbox daemon session owner.

use crate::{SandboxControlCommand, SandboxControlResponse, SandboxControlStatus};

pub trait SandboxControlPort: Send + 'static {
    fn execute(&mut self, command: SandboxControlCommand) -> SandboxControlResponse;

    fn status(&self) -> SandboxControlStatus;
}

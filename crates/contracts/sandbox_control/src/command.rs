//! Bounded commands accepted by the Guest-local sandbox control server.

use crate::SandboxEndpoint;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxConnectCommand {
    endpoint: SandboxEndpoint,
}

impl SandboxConnectCommand {
    pub const fn new(endpoint: SandboxEndpoint) -> Self {
        Self { endpoint }
    }

    pub const fn endpoint(self) -> SandboxEndpoint {
        self.endpoint
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxControlCommand {
    Connect(SandboxConnectCommand),
}

//! Read-only daemon and transport-session status exposed across the control boundary.

use crate::SandboxEndpoint;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxDaemonState {
    Ready,
    Stopping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxControlStatus {
    pub daemon: SandboxDaemonState,
    pub connection: SandboxConnectionState,
    pub endpoint: Option<SandboxEndpoint>,
    pub sb_id: u32,
    pub connection_generation: u64,
    pub publication_enabled: bool,
}

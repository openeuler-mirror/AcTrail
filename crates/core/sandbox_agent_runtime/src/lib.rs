//! Guest-local collector, sampler, batching, and VSOCK sender lifecycle.

mod config;
mod daemon;
mod delivery;
mod ports;
mod session;
mod status;

pub use config::SandboxAgentConfig;
pub use daemon::{SandboxAgentControlHandle, SandboxAgentDaemon};
pub use ports::{GuestResourceSource, ProcessIoSource, SandboxConnection, SandboxTransportFactory};
pub use status::SandboxAgentSnapshot;

//! Guest-local collector, sampler, batching, and VSOCK sender lifecycle.

mod config;
mod ports;
mod runtime;
mod sender;
mod status;

pub use config::SandboxAgentConfig;
pub use ports::{GuestResourceSource, ProcessIoSource, SandboxConnection, SandboxTransport};
pub use runtime::SandboxAgent;
pub use status::SandboxAgentSnapshot;

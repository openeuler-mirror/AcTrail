//! Guest-local Unix-domain-socket adapter for sandbox daemon control.

mod client;
mod codec;
mod connection;
mod dispatcher;
mod error;
mod runtime;
mod server;

pub use client::{SandboxControlUdsClient, SandboxControlUdsClientConfig};
pub use codec::SandboxControlCodec;
pub use connection::SandboxControlConnectionLimits;
pub use error::{SandboxControlUdsError, SandboxControlUdsStage};
pub use server::{
    SandboxControlServerHandle, SandboxControlUdsServer, SandboxControlUdsServerConfig,
};

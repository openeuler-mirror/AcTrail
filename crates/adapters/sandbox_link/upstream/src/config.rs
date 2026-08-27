use std::net::SocketAddr;
use std::time::Duration;

use sandbox_upstream_contract::MAX_FRAME_BYTES;

use crate::ServerStartError;

const MIN_READ_BUFFER_BYTES: usize = 1024;
const MIN_THREAD_STACK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpstreamServerConfig {
    pub listen_addr: SocketAddr,
    pub max_connections: u32,
    pub accept_poll_interval: Duration,
    pub connection_poll_interval: Duration,
    pub connection_idle_timeout: Duration,
    pub write_timeout: Duration,
    pub read_buffer_bytes: usize,
    pub connection_thread_stack_bytes: usize,
}

impl UpstreamServerConfig {
    pub(crate) fn validate(&self) -> Result<(), ServerStartError> {
        if self.max_connections == 0 {
            return Err(ServerStartError::config("max_connections must be positive"));
        }
        if self.accept_poll_interval.is_zero() {
            return Err(ServerStartError::config(
                "accept_poll_interval must be positive",
            ));
        }
        if self.connection_poll_interval.is_zero() {
            return Err(ServerStartError::config(
                "connection_poll_interval must be positive",
            ));
        }
        if self.connection_idle_timeout.is_zero() {
            return Err(ServerStartError::config(
                "connection_idle_timeout must be positive",
            ));
        }
        if self.connection_poll_interval > self.connection_idle_timeout {
            return Err(ServerStartError::config(
                "connection_poll_interval must not exceed connection_idle_timeout",
            ));
        }
        if self.write_timeout.is_zero() {
            return Err(ServerStartError::config("write_timeout must be positive"));
        }
        if !(MIN_READ_BUFFER_BYTES..=MAX_FRAME_BYTES).contains(&self.read_buffer_bytes) {
            return Err(ServerStartError::config(format!(
                "read_buffer_bytes must be within {MIN_READ_BUFFER_BYTES}..={MAX_FRAME_BYTES}"
            )));
        }
        if self.connection_thread_stack_bytes < MIN_THREAD_STACK_BYTES {
            return Err(ServerStartError::config(format!(
                "connection_thread_stack_bytes must be at least {MIN_THREAD_STACK_BYTES}"
            )));
        }
        Ok(())
    }
}

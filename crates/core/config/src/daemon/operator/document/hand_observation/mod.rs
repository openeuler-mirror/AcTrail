use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::daemon::HandObservationConfig;

use super::require_positive_u32;
use super::require_positive_u64;

const DEFAULT_LISTEN_PORT: u16 = 9472;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct HandObservationDocument {
    pub enabled: bool,
    pub listen_addr: String,
    pub max_gateway_connections: u32,
    pub accept_poll_interval_ms: u64,
    pub connection_poll_interval_ms: u64,
    pub connection_idle_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub read_buffer_bytes: usize,
    pub connection_thread_stack_bytes: usize,
}

impl Default for HandObservationDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_LISTEN_PORT)
                .to_string(),
            max_gateway_connections: 64,
            accept_poll_interval_ms: 20,
            connection_poll_interval_ms: 250,
            connection_idle_timeout_ms: 30_000,
            write_timeout_ms: 5_000,
            read_buffer_bytes: 65_536,
            connection_thread_stack_bytes: 524_288,
        }
    }
}

impl HandObservationDocument {
    pub(super) fn from_config(config: &HandObservationConfig) -> Self {
        Self {
            enabled: config.enabled,
            listen_addr: config.listen_addr.to_string(),
            max_gateway_connections: config.max_gateway_connections,
            accept_poll_interval_ms: config.accept_poll_interval_ms,
            connection_poll_interval_ms: config.connection_poll_interval_ms,
            connection_idle_timeout_ms: config.connection_idle_timeout_ms,
            write_timeout_ms: config.write_timeout_ms,
            read_buffer_bytes: config.read_buffer_bytes,
            connection_thread_stack_bytes: config.connection_thread_stack_bytes,
        }
    }

    pub(super) fn to_config(&self) -> Result<HandObservationConfig, String> {
        let listen_addr = self
            .listen_addr
            .parse::<SocketAddr>()
            .map_err(|error| format!("hand_observation.listen_addr is invalid: {error}"))?;
        if self.read_buffer_bytes == 0 || self.connection_thread_stack_bytes == 0 {
            return Err(
                "hand_observation read buffer and connection thread stack must be positive"
                    .to_string(),
            );
        }
        Ok(HandObservationConfig {
            enabled: self.enabled,
            listen_addr,
            max_gateway_connections: require_positive_u32(
                "hand_observation.max_gateway_connections",
                self.max_gateway_connections,
            )?,
            accept_poll_interval_ms: require_positive_u64(
                "hand_observation.accept_poll_interval_ms",
                self.accept_poll_interval_ms,
            )?,
            connection_poll_interval_ms: require_positive_u64(
                "hand_observation.connection_poll_interval_ms",
                self.connection_poll_interval_ms,
            )?,
            connection_idle_timeout_ms: require_positive_u64(
                "hand_observation.connection_idle_timeout_ms",
                self.connection_idle_timeout_ms,
            )?,
            write_timeout_ms: require_positive_u64(
                "hand_observation.write_timeout_ms",
                self.write_timeout_ms,
            )?,
            read_buffer_bytes: self.read_buffer_bytes,
            connection_thread_stack_bytes: self.connection_thread_stack_bytes,
        })
    }
}

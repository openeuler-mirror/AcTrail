use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use sandbox_vsock_transport::VsockListenerConfig;

#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub listener: VsockListenerConfig,
    pub daemon_address: SocketAddr,
    pub max_sb_connections: usize,
    pub per_sb_forward_quota: usize,
    pub outbound_queue_capacity: usize,
    pub upstream_heartbeat_interval: Duration,
    pub sb_peer_idle_timeout: Duration,
    pub io_timeout: Duration,
    pub reconnect_interval: Duration,
    pub accept_poll_interval: Duration,
    pub connection_thread_stack_bytes: usize,
}

impl GatewayConfig {
    pub fn validate(&self) -> io::Result<()> {
        if self.max_sb_connections == 0
            || self.per_sb_forward_quota == 0
            || self.outbound_queue_capacity == 0
            || self.connection_thread_stack_bytes == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "gateway capacities and thread stack must be positive",
            ));
        }
        for (name, value) in [
            (
                "upstream_heartbeat_interval",
                self.upstream_heartbeat_interval,
            ),
            ("sb_peer_idle_timeout", self.sb_peer_idle_timeout),
            ("io_timeout", self.io_timeout),
            ("reconnect_interval", self.reconnect_interval),
            ("accept_poll_interval", self.accept_poll_interval),
        ] {
            if value.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("gateway {name} must be positive"),
                ));
            }
        }
        let reserved_capacity = self
            .max_sb_connections
            .checked_mul(self.per_sb_forward_quota)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "gateway reserved forward capacity overflow",
                )
            })?;
        if reserved_capacity > self.outbound_queue_capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "global outbound queue must cover every SB reserved forward quota",
            ));
        }
        Ok(())
    }
}

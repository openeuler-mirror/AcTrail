use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vsock_gateway_runtime::GatewayConfig;

use super::backend::{GatewayBackend, ListenerSection};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAppConfig {
    vsock: VsockSection,
    upstream: UpstreamSection,
    runtime: RuntimeSection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VsockSection {
    backlog: u32,
    listener: ListenerSection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpstreamSection {
    daemon_address: SocketAddr,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSection {
    max_sb_connections: usize,
    per_sb_forward_quota: usize,
    outbound_queue_capacity: usize,
    upstream_heartbeat_interval_ms: u64,
    sb_peer_idle_timeout_ms: u64,
    io_timeout_ms: u64,
    reconnect_interval_ms: u64,
    accept_poll_interval_ms: u64,
    connection_thread_stack_bytes: usize,
}

#[derive(Debug, Default)]
pub struct GatewayConfigOverrides {
    backend: Option<GatewayBackend>,
    uds_path: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    cid: Option<u32>,
    port: Option<u32>,
    daemon_address: Option<SocketAddr>,
}

impl GatewayConfigOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backend(mut self, backend: GatewayBackend) -> Self {
        self.backend = Some(backend);
        self
    }

    pub fn with_socket_path(mut self, socket_path: PathBuf) -> Self {
        self.socket_path = Some(socket_path);
        self
    }

    pub fn with_uds_path(mut self, uds_path: PathBuf) -> Self {
        self.uds_path = Some(uds_path);
        self
    }

    pub fn with_cid(mut self, cid: u32) -> Self {
        self.cid = Some(cid);
        self
    }

    pub fn with_port(mut self, port: u32) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_daemon_address(mut self, daemon_address: SocketAddr) -> Self {
        self.daemon_address = Some(daemon_address);
        self
    }
}

impl GatewayAppConfig {
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn into_runtime(self) -> io::Result<GatewayConfig> {
        let listener = self.vsock.listener.resolve(self.validated_backlog()?)?;
        let config = GatewayConfig {
            listener,
            daemon_address: self.upstream.daemon_address,
            max_sb_connections: self.runtime.max_sb_connections,
            per_sb_forward_quota: self.runtime.per_sb_forward_quota,
            outbound_queue_capacity: self.runtime.outbound_queue_capacity,
            upstream_heartbeat_interval: Duration::from_millis(
                self.runtime.upstream_heartbeat_interval_ms,
            ),
            sb_peer_idle_timeout: Duration::from_millis(self.runtime.sb_peer_idle_timeout_ms),
            io_timeout: Duration::from_millis(self.runtime.io_timeout_ms),
            reconnect_interval: Duration::from_millis(self.runtime.reconnect_interval_ms),
            accept_poll_interval: Duration::from_millis(self.runtime.accept_poll_interval_ms),
            connection_thread_stack_bytes: self.runtime.connection_thread_stack_bytes,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn write_default(
        path: &Path,
        overrides: GatewayConfigOverrides,
        force: bool,
    ) -> io::Result<()> {
        let mut config = Self::default_config();
        if let Some(backend) = overrides.backend {
            config.vsock.listener = ListenerSection::for_backend(backend);
        }
        if let Some(uds_path) = overrides.uds_path {
            config.vsock.listener.set_uds_path(uds_path)?;
        }
        if let Some(socket_path) = overrides.socket_path {
            config.vsock.listener.set_socket_path(socket_path)?;
        }
        if let Some(cid) = overrides.cid {
            config.vsock.listener.set_cid(cid)?;
        }
        if let Some(port) = overrides.port {
            config.vsock.listener.set_port(port)?;
        }
        if let Some(daemon_address) = overrides.daemon_address {
            config.upstream.daemon_address = daemon_address;
        }
        config.clone().into_runtime()?;
        let text = toml::to_string_pretty(&config)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        toml::from_str::<Self>(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .into_runtime()?;
        let mut options = OpenOptions::new();
        options.write(true);
        if force {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        drop(file);
        Self::load(path)?.into_runtime().map(|_| ())
    }

    fn validated_backlog(&self) -> io::Result<u32> {
        if self.vsock.backlog == 0 || self.vsock.backlog > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VSOCK backlog must fit a positive i32",
            ));
        }
        Ok(self.vsock.backlog)
    }

    fn default_config() -> Self {
        Self {
            vsock: VsockSection {
                backlog: 128,
                listener: ListenerSection::default_firecracker(),
            },
            upstream: UpstreamSection {
                daemon_address: "127.0.0.1:9472"
                    .parse()
                    .expect("hard-coded gateway daemon address must parse"),
            },
            runtime: RuntimeSection {
                max_sb_connections: 64,
                per_sb_forward_quota: 16,
                outbound_queue_capacity: 1_024,
                upstream_heartbeat_interval_ms: 5_000,
                sb_peer_idle_timeout_ms: 15_000,
                io_timeout_ms: 1_000,
                reconnect_interval_ms: 1_000,
                accept_poll_interval_ms: 20,
                connection_thread_stack_bytes: 524_288,
            },
        }
    }
}

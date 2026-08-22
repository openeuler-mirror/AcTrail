use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_vsock_transport::VsockListenerConfig;
use serde::{Deserialize, Serialize};
use vsock_gateway_runtime::GatewayConfig;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAppConfig {
    pub vsock: VsockSection,
    pub upstream: UpstreamSection,
    pub runtime: RuntimeSection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VsockSection {
    pub backend: String,
    pub cid: Option<u32>,
    pub port: Option<u32>,
    pub socket_path: Option<PathBuf>,
    pub backlog: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamSection {
    pub daemon_address: SocketAddr,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSection {
    pub max_sb_connections: usize,
    pub per_sb_forward_quota: usize,
    pub outbound_queue_capacity: usize,
    pub upstream_heartbeat_interval_ms: u64,
    pub sb_peer_idle_timeout_ms: u64,
    pub io_timeout_ms: u64,
    pub reconnect_interval_ms: u64,
    pub accept_poll_interval_ms: u64,
    pub connection_thread_stack_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum GatewayBackend {
    Native,
    CloudHypervisor,
}

#[derive(Debug, Default)]
pub struct GatewayConfigOverrides {
    backend: Option<GatewayBackend>,
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
        let listener = match self.vsock.backend.as_str() {
            "native" => {
                if self.vsock.socket_path.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "native VSOCK does not accept socket_path",
                    ));
                }
                let port = self.vsock.port.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "native VSOCK requires port")
                })?;
                if port == libc::VMADDR_PORT_ANY {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "native VSOCK port must be concrete",
                    ));
                }
                VsockListenerConfig::Native {
                    cid: self.vsock.cid.ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "native VSOCK requires cid")
                    })?,
                    port,
                    backlog: self.validated_backlog()?,
                }
            }
            "cloud-hypervisor" => VsockListenerConfig::CloudHypervisor {
                socket_path: self.validated_cloud_hypervisor_path()?,
                backlog: self.validated_backlog()?,
            },
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported VSOCK backend {other}"),
                ));
            }
        };
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
            match backend {
                GatewayBackend::Native => {
                    config.vsock.backend = "native".to_string();
                    config.vsock.socket_path = None;
                }
                GatewayBackend::CloudHypervisor => {
                    config.vsock.backend = "cloud-hypervisor".to_string();
                    config.vsock.cid = None;
                    config.vsock.port = None;
                }
            }
        }
        if let Some(socket_path) = overrides.socket_path {
            config.vsock.socket_path = Some(socket_path);
        }
        if let Some(cid) = overrides.cid {
            config.vsock.cid = Some(cid);
        }
        if let Some(port) = overrides.port {
            config.vsock.port = Some(port);
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

    fn validated_cloud_hypervisor_path(&self) -> io::Result<PathBuf> {
        if self.vsock.cid.is_some() || self.vsock.port.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cloud Hypervisor VSOCK does not accept native cid or port",
            ));
        }
        let path = self.vsock.socket_path.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cloud Hypervisor VSOCK requires socket_path",
            )
        })?;
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cloud Hypervisor VSOCK socket_path must be absolute",
            ));
        }
        Ok(path)
    }

    fn default_config() -> Self {
        Self {
            vsock: VsockSection {
                backend: "native".to_string(),
                cid: Some(libc::VMADDR_CID_ANY),
                port: Some(43_182),
                socket_path: None,
                backlog: 128,
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

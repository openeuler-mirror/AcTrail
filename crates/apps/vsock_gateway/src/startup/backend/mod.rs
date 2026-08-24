mod cloud_hypervisor;
mod firecracker;
mod native;

use std::io;
use std::path::PathBuf;

use sandbox_vsock_transport::VsockListenerConfig;
use serde::{Deserialize, Serialize};

use self::cloud_hypervisor::CloudHypervisorEndpoint;
use self::firecracker::FirecrackerEndpoint;
use self::native::NativeEndpoint;

#[derive(Clone, Copy, Debug)]
pub enum GatewayBackend {
    Firecracker,
    Native,
    CloudHypervisor,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "backend", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum ListenerSection {
    Firecracker { uds_path: PathBuf, port: u32 },
    Native { cid: u32, port: u32 },
    CloudHypervisor { socket_path: PathBuf },
}

impl ListenerSection {
    pub(super) fn default_firecracker() -> Self {
        Self::Firecracker {
            uds_path: PathBuf::from("/run/firecracker/actrail/vsock.sock"),
            port: 43_182,
        }
    }

    pub(super) fn for_backend(backend: GatewayBackend) -> Self {
        match backend {
            GatewayBackend::Firecracker => Self::default_firecracker(),
            GatewayBackend::Native => Self::Native {
                cid: libc::VMADDR_CID_ANY,
                port: 43_182,
            },
            GatewayBackend::CloudHypervisor => Self::CloudHypervisor {
                socket_path: PathBuf::new(),
            },
        }
    }

    pub(super) fn resolve(&self, backlog: u32) -> io::Result<VsockListenerConfig> {
        match self {
            Self::Firecracker { uds_path, port } => {
                FirecrackerEndpoint::resolve(uds_path, *port)?.listener(backlog)
            }
            Self::Native { cid, port } => NativeEndpoint::new(*cid, *port).listener(backlog),
            Self::CloudHypervisor { socket_path } => {
                CloudHypervisorEndpoint::new(socket_path)?.listener(backlog)
            }
        }
    }

    pub(super) fn set_uds_path(&mut self, uds_path: PathBuf) -> io::Result<()> {
        match self {
            Self::Firecracker {
                uds_path: configured,
                ..
            } => {
                *configured = uds_path;
                Ok(())
            }
            _ => Err(Self::option_error("--uds-path", "firecracker")),
        }
    }

    pub(super) fn set_socket_path(&mut self, socket_path: PathBuf) -> io::Result<()> {
        match self {
            Self::CloudHypervisor {
                socket_path: configured,
            } => {
                *configured = socket_path;
                Ok(())
            }
            _ => Err(Self::option_error("--socket-path", "cloud-hypervisor")),
        }
    }

    pub(super) fn set_cid(&mut self, cid: u32) -> io::Result<()> {
        match self {
            Self::Native {
                cid: configured, ..
            } => {
                *configured = cid;
                Ok(())
            }
            _ => Err(Self::option_error("--cid", "native")),
        }
    }

    pub(super) fn set_port(&mut self, port: u32) -> io::Result<()> {
        match self {
            Self::Firecracker {
                port: configured, ..
            }
            | Self::Native {
                port: configured, ..
            } => {
                *configured = port;
                Ok(())
            }
            Self::CloudHypervisor { .. } => {
                Err(Self::option_error("--port", "firecracker or native"))
            }
        }
    }

    fn option_error(option: &str, backend: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{option} requires the {backend} VSOCK backend"),
        )
    }
}

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use sandbox_vsock_transport::VsockListenerConfig;

pub(super) struct FirecrackerEndpoint {
    socket_path: PathBuf,
}

impl FirecrackerEndpoint {
    pub(super) fn resolve(uds_path: &Path, port: u32) -> io::Result<Self> {
        if !uds_path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Firecracker VSOCK uds_path must be absolute",
            ));
        }
        if uds_path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Firecracker VSOCK uds_path must not be empty",
            ));
        }
        if port == libc::VMADDR_PORT_ANY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Firecracker VSOCK port must be concrete",
            ));
        }
        let mut endpoint = OsString::from(uds_path.as_os_str());
        endpoint.push(format!("_{port}"));
        Ok(Self {
            socket_path: PathBuf::from(endpoint),
        })
    }

    pub(super) fn listener(self, backlog: u32) -> io::Result<VsockListenerConfig> {
        Ok(VsockListenerConfig::UnixSocket {
            socket_path: self.socket_path,
            backlog,
        })
    }
}

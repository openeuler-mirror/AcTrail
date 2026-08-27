use std::io;
use std::path::{Path, PathBuf};

use sandbox_vsock_transport::VsockListenerConfig;

pub(super) struct CloudHypervisorEndpoint {
    socket_path: PathBuf,
}

impl CloudHypervisorEndpoint {
    pub(super) fn new(socket_path: &Path) -> io::Result<Self> {
        if !socket_path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cloud Hypervisor VSOCK socket_path must be absolute",
            ));
        }
        if socket_path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cloud Hypervisor VSOCK socket_path must not be empty",
            ));
        }
        Ok(Self {
            socket_path: socket_path.to_path_buf(),
        })
    }

    pub(super) fn listener(self, backlog: u32) -> io::Result<VsockListenerConfig> {
        Ok(VsockListenerConfig::UnixSocket {
            socket_path: self.socket_path,
            backlog,
        })
    }
}

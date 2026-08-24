use std::io;
use std::time::Duration;

use sandbox_agent_runtime::{SandboxConnection, SandboxTransportFactory as SandboxTransportPort};
use sandbox_control::SandboxEndpoint;

use crate::{VsockConnection, kernel_vsock};

#[derive(Clone, Copy, Debug)]
pub struct VsockTransportConfig {
    pub io_timeout: Duration,
}

#[derive(Debug)]
pub struct VsockTransportFactory {
    config: VsockTransportConfig,
}

impl VsockTransportFactory {
    pub fn new(config: VsockTransportConfig) -> io::Result<Self> {
        if config.io_timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VSOCK I/O timeout must be positive",
            ));
        }
        Ok(Self { config })
    }

    pub fn connect(&self, endpoint: SandboxEndpoint) -> io::Result<VsockConnection> {
        let stream =
            kernel_vsock::connect(endpoint.host_cid(), endpoint.port(), self.config.io_timeout)?;
        let connection = VsockConnection::kernel_vsock(stream);
        connection.set_timeouts(self.config.io_timeout)?;
        Ok(connection)
    }
}

impl SandboxTransportPort for VsockTransportFactory {
    fn connect(&self, endpoint: SandboxEndpoint) -> io::Result<Box<dyn SandboxConnection>> {
        self.connect(endpoint)
            .map(|connection| Box::new(connection) as Box<_>)
    }
}

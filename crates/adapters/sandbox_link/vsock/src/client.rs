use std::io;
use std::time::Duration;

use crate::{VsockConnection, native};

#[derive(Clone, Copy, Debug)]
pub struct VsockClientConfig {
    pub host_cid: u32,
    pub port: u32,
    pub io_timeout: Duration,
}

#[derive(Debug)]
pub struct VsockClient {
    config: VsockClientConfig,
}

impl VsockClient {
    pub fn new(config: VsockClientConfig) -> io::Result<Self> {
        if config.host_cid == libc::VMADDR_CID_ANY || config.port == libc::VMADDR_PORT_ANY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VSOCK client CID and port must be concrete",
            ));
        }
        if config.io_timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VSOCK I/O timeout must be positive",
            ));
        }
        Ok(Self { config })
    }

    pub fn connect(&self) -> io::Result<VsockConnection> {
        let stream = native::connect(self.config.host_cid, self.config.port)?;
        let connection = VsockConnection::native(stream, self.config.host_cid, self.config.port);
        connection.set_timeouts(self.config.io_timeout)?;
        Ok(connection)
    }
}

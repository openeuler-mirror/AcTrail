use std::io;

use sandbox_vsock_transport::VsockListenerConfig;

pub(super) struct NativeEndpoint {
    cid: u32,
    port: u32,
}

impl NativeEndpoint {
    pub(super) fn new(cid: u32, port: u32) -> Self {
        Self { cid, port }
    }

    pub(super) fn listener(self, backlog: u32) -> io::Result<VsockListenerConfig> {
        if self.port == libc::VMADDR_PORT_ANY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native VSOCK port must be concrete",
            ));
        }
        Ok(VsockListenerConfig::KernelVsock {
            cid: self.cid,
            port: self.port,
            backlog,
        })
    }
}

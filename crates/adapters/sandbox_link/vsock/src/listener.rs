use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use crate::{VsockConnection, kernel_vsock, unix_stream};

#[derive(Clone, Debug)]
pub enum VsockListenerConfig {
    KernelVsock { cid: u32, port: u32, backlog: u32 },
    UnixSocket { socket_path: PathBuf, backlog: u32 },
}

impl VsockListenerConfig {
    pub fn validate(&self) -> io::Result<()> {
        let (port, backlog) = match self {
            Self::KernelVsock { port, backlog, .. } => (Some(*port), *backlog),
            Self::UnixSocket {
                socket_path,
                backlog,
            } => {
                unix_stream::validate_path(socket_path)?;
                (None, *backlog)
            }
        };
        if port == Some(libc::VMADDR_PORT_ANY) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "kernel VSOCK port must be concrete",
            ));
        }
        if backlog == 0 || backlog > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VSOCK backlog must fit a positive i32",
            ));
        }
        Ok(())
    }
}

pub enum VsockListener {
    KernelVsock(File),
    UnixSocket {
        listener: UnixListener,
        socket_path: PathBuf,
    },
}

impl VsockListener {
    pub fn bind(config: &VsockListenerConfig) -> io::Result<Self> {
        config.validate()?;
        match config {
            VsockListenerConfig::KernelVsock { cid, port, backlog } => {
                let listener = kernel_vsock::bind(*cid, *port, *backlog)?;
                Ok(Self::KernelVsock(listener))
            }
            VsockListenerConfig::UnixSocket {
                socket_path,
                backlog,
            } => {
                let listener = unix_stream::bind(socket_path, *backlog)?;
                Ok(Self::UnixSocket {
                    listener,
                    socket_path: socket_path.clone(),
                })
            }
        }
    }

    pub fn set_nonblocking(&self, enabled: bool) -> io::Result<()> {
        match self {
            Self::KernelVsock(listener) => {
                kernel_vsock::set_nonblocking(listener.as_raw_fd(), enabled)
            }
            Self::UnixSocket { listener, .. } => listener.set_nonblocking(enabled),
        }
    }

    pub fn accept(&self) -> io::Result<VsockConnection> {
        match self {
            Self::KernelVsock(listener) => {
                let (stream, _, _) = kernel_vsock::accept(listener.as_raw_fd())?;
                Ok(VsockConnection::kernel_vsock(stream))
            }
            Self::UnixSocket { listener, .. } => {
                let (stream, _) = listener.accept()?;
                Ok(VsockConnection::unix_socket(stream))
            }
        }
    }
}

impl Drop for VsockListener {
    fn drop(&mut self) {
        if let Self::UnixSocket { socket_path, .. } = self {
            let _ = std::fs::remove_file(socket_path);
        }
    }
}

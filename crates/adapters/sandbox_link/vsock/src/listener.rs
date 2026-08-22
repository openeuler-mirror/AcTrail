use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use crate::{VsockConnection, native};

#[derive(Clone, Debug)]
pub enum VsockListenerConfig {
    Native { cid: u32, port: u32, backlog: u32 },
    CloudHypervisor { socket_path: PathBuf, backlog: u32 },
}

pub enum VsockListener {
    Native(File),
    CloudHypervisor {
        listener: UnixListener,
        socket_path: PathBuf,
    },
}

impl VsockListener {
    pub fn bind(config: &VsockListenerConfig) -> io::Result<Self> {
        match config {
            VsockListenerConfig::Native { cid, port, backlog } => {
                if *port == libc::VMADDR_PORT_ANY || *backlog == 0 || *backlog > i32::MAX as u32 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "VSOCK port must be concrete and backlog must fit a positive i32",
                    ));
                }
                let listener = native::bind(*cid, *port, *backlog)?;
                Ok(Self::Native(listener))
            }
            VsockListenerConfig::CloudHypervisor {
                socket_path,
                backlog,
            } => {
                if *backlog == 0 || *backlog > i32::MAX as u32 || socket_path.as_os_str().is_empty()
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Cloud Hypervisor socket path and positive i32 backlog are required",
                    ));
                }
                let listener = native::bind_unix(socket_path, *backlog)?;
                Ok(Self::CloudHypervisor {
                    listener,
                    socket_path: socket_path.clone(),
                })
            }
        }
    }

    pub fn set_nonblocking(&self, enabled: bool) -> io::Result<()> {
        match self {
            Self::Native(listener) => native::set_nonblocking(listener.as_raw_fd(), enabled),
            Self::CloudHypervisor { listener, .. } => listener.set_nonblocking(enabled),
        }
    }

    pub fn accept(&self) -> io::Result<VsockConnection> {
        match self {
            Self::Native(listener) => {
                let (stream, cid, port) = native::accept(listener.as_raw_fd())?;
                Ok(VsockConnection::native(stream, cid, port))
            }
            Self::CloudHypervisor {
                listener,
                socket_path,
            } => {
                let (stream, _) = listener.accept()?;
                Ok(VsockConnection::cloud_hypervisor(
                    stream,
                    socket_path.display().to_string(),
                ))
            }
        }
    }
}

impl Drop for VsockListener {
    fn drop(&mut self) {
        if let Self::CloudHypervisor { socket_path, .. } = self {
            let _ = std::fs::remove_file(socket_path);
        }
    }
}

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::native;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerAddress {
    Native { cid: u32, port: u32 },
    CloudHypervisor { socket_path: String },
}

pub enum VsockConnection {
    Native {
        stream: File,
        peer: PeerAddress,
    },
    CloudHypervisor {
        stream: UnixStream,
        peer: PeerAddress,
    },
}

impl VsockConnection {
    pub(crate) fn native(stream: File, cid: u32, port: u32) -> Self {
        Self::Native {
            stream,
            peer: PeerAddress::Native { cid, port },
        }
    }

    pub(crate) fn cloud_hypervisor(stream: UnixStream, socket_path: String) -> Self {
        Self::CloudHypervisor {
            stream,
            peer: PeerAddress::CloudHypervisor { socket_path },
        }
    }

    pub fn peer(&self) -> &PeerAddress {
        match self {
            Self::Native { peer, .. } | Self::CloudHypervisor { peer, .. } => peer,
        }
    }

    pub fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
        match self {
            Self::Native { stream, .. } => native::set_timeouts(stream.as_raw_fd(), timeout),
            Self::CloudHypervisor { stream, .. } => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))
            }
        }
    }
}

impl Read for VsockConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Native { stream, .. } => stream.read(buffer),
            Self::CloudHypervisor { stream, .. } => stream.read(buffer),
        }
    }
}

impl Write for VsockConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Native { stream, .. } => stream.write(buffer),
            Self::CloudHypervisor { stream, .. } => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Native { stream, .. } => stream.flush(),
            Self::CloudHypervisor { stream, .. } => stream.flush(),
        }
    }
}

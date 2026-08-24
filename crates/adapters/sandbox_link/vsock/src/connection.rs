use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::kernel_vsock;

pub enum VsockConnection {
    KernelVsock { stream: File },
    UnixSocket { stream: UnixStream },
}

impl VsockConnection {
    pub(crate) fn kernel_vsock(stream: File) -> Self {
        Self::KernelVsock { stream }
    }

    pub(crate) fn unix_socket(stream: UnixStream) -> Self {
        Self::UnixSocket { stream }
    }

    pub fn set_timeouts(&self, timeout: Duration) -> io::Result<()> {
        match self {
            Self::KernelVsock { stream, .. } => {
                kernel_vsock::set_timeouts(stream.as_raw_fd(), timeout)
            }
            Self::UnixSocket { stream, .. } => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))
            }
        }
    }
}

impl Read for VsockConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::KernelVsock { stream, .. } => stream.read(buffer),
            Self::UnixSocket { stream, .. } => stream.read(buffer),
        }
    }
}

impl Write for VsockConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::KernelVsock { stream, .. } => stream.write(buffer),
            Self::UnixSocket { stream, .. } => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::KernelVsock { stream, .. } => stream.flush(),
            Self::UnixSocket { stream, .. } => stream.flush(),
        }
    }
}

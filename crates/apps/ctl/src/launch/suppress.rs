//! Launch-owned fd setup for observation suppression.

use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use model_core::process::{InitialSuppressedFd, SuppressedFdPurpose};

pub(super) struct InheritableSuppressedFd {
    stream: UnixStream,
    purpose: SuppressedFdPurpose,
}

impl InheritableSuppressedFd {
    pub(super) fn connect_unix_socket(
        path: &Path,
        purpose: SuppressedFdPurpose,
    ) -> Result<Self, String> {
        let stream = UnixStream::connect(path)
            .map_err(|error| format!("connect suppressed fd socket {}: {error}", path.display()))?;
        set_close_on_exec(stream.as_raw_fd(), false)?;
        Ok(Self { stream, purpose })
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    pub(super) fn initial_suppressed_fd(&self) -> InitialSuppressedFd {
        InitialSuppressedFd {
            fd: self.raw_fd(),
            purpose: self.purpose,
        }
    }
}

fn set_close_on_exec(fd: RawFd, close_on_exec: bool) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(format!(
            "read suppressed fd flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    let next = if close_on_exec {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, next) } < 0 {
        return Err(format!(
            "update suppressed fd close-on-exec: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

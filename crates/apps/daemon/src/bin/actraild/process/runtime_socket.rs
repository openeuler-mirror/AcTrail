use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::Path;

pub(super) struct RuntimeSocket<'a> {
    label: &'a str,
    path: &'a Path,
}

impl<'a> RuntimeSocket<'a> {
    pub(super) fn new(label: &'a str, path: &'a Path) -> Self {
        Self { label, path }
    }

    pub(super) fn ensure_available(&self) -> Result<(), String> {
        if !self.inspect()? {
            return Ok(());
        }
        match self.try_connect() {
            Ok(_) => Err(format!(
                "{} socket {} already has an active listener",
                self.label,
                self.path.display()
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                super::remove_runtime_file(self.path)
            }
            Err(error) => Err(format!(
                "{} socket {} exists but could not be verified stale: {error}; remove it explicitly",
                self.label,
                self.path.display()
            )),
        }
    }

    pub(super) fn listener(&self) -> Result<UnixStream, String> {
        if !self.inspect()? {
            return Err(format!(
                "{} socket {} is missing",
                self.label,
                self.path.display()
            ));
        }
        self.try_connect().map_err(|error| {
            format!(
                "connect {} socket {}: {error}",
                self.label,
                self.path.display()
            )
        })
    }

    fn inspect(&self) -> Result<bool, String> {
        let metadata = match fs::symlink_metadata(self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "inspect {} socket {}: {error}",
                    self.label,
                    self.path.display()
                ));
            }
        };
        if !metadata.file_type().is_socket() {
            return Err(format!(
                "{} socket path {} exists but is not a Unix socket; remove it explicitly",
                self.label,
                self.path.display()
            ));
        }
        Ok(true)
    }

    fn try_connect(&self) -> io::Result<UnixStream> {
        let bytes = self.path.as_os_str().as_bytes();
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Unix socket path",
            ));
        }
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
            *target = *byte as libc::c_char;
        }
        let raw = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
        let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
        let result = unsafe {
            libc::connect(
                descriptor.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                length as libc::socklen_t,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(UnixStream::from(descriptor))
    }
}

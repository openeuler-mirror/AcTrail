use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

pub(super) struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    pub(super) fn acquire(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result < 0 {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(
                if error.kind() == io::ErrorKind::WouldBlock {
                    io::ErrorKind::AlreadyExists
                } else {
                    error.kind()
                },
                format!("another actrail-sb owns {}: {error}", path.display()),
            ));
        }
        Ok(Self { _file: file })
    }
}

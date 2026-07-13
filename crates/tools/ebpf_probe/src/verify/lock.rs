//! Cross-process isolation for live verification paths.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(super) struct LiveVerificationLock {
    _file: File,
}

impl LiveVerificationLock {
    pub(super) fn acquire(storage_path: &Path) -> Result<Self, String> {
        let lock_path = Self::path_for(storage_path);
        if let Some(parent) = lock_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "create verify-live lock directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&lock_path)
            .map_err(|error| format!("open verify-live lock {}: {error}", lock_path.display()))?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error
                .raw_os_error()
                .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
            {
                let mut owner = String::new();
                file.seek(SeekFrom::Start(0)).map_err(|seek_error| {
                    format!(
                        "read verify-live lock {}: {seek_error}",
                        lock_path.display()
                    )
                })?;
                file.read_to_string(&mut owner).map_err(|read_error| {
                    format!(
                        "read verify-live lock {}: {read_error}",
                        lock_path.display()
                    )
                })?;
                let owner = owner.trim();
                let suffix = if owner.is_empty() {
                    String::new()
                } else {
                    format!(" ({owner})")
                };
                return Err(format!(
                    "another verify-live owns {}{suffix}",
                    lock_path.display()
                ));
            }
            return Err(format!(
                "lock verify-live path {}: {error}",
                lock_path.display()
            ));
        }
        file.set_len(0).map_err(|error| {
            format!("truncate verify-live lock {}: {error}", lock_path.display())
        })?;
        write!(
            file,
            "pid={} cwd={}",
            std::process::id(),
            Self::display_cwd()
        )
        .map_err(|error| format!("write verify-live lock {}: {error}", lock_path.display()))?;
        file.sync_data()
            .map_err(|error| format!("sync verify-live lock {}: {error}", lock_path.display()))?;
        Ok(Self { _file: file })
    }

    fn path_for(storage_path: &Path) -> PathBuf {
        let mut value: OsString = storage_path.as_os_str().to_owned();
        value.push(".verify-live.lock");
        PathBuf::from(value)
    }

    fn display_cwd() -> String {
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("<unavailable:{error}>"))
    }
}

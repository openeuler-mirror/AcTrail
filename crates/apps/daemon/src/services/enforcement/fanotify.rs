//! Linux fanotify permission-event bindings.

use std::ffi::CString;
use std::io;
use std::mem::size_of;
use std::os::fd::RawFd;
use std::path::Path;

use crate::services::enforcement::rules::FileKey;

pub(super) struct FanotifyHandle {
    fd: RawFd,
    event_buffer_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PermissionMetadata {
    pub fd: RawFd,
    pub pid: i32,
    pub mask: u64,
}

impl FanotifyHandle {
    pub(super) fn new(event_buffer_bytes: u32) -> Result<Self, String> {
        let fd = unsafe {
            libc::fanotify_init(
                libc::FAN_CLASS_PRE_CONTENT | libc::FAN_CLOEXEC | libc::FAN_NONBLOCK,
                (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_LARGEFILE) as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(format!("fanotify_init: {}", io::Error::last_os_error()));
        }
        Ok(Self {
            fd,
            event_buffer_bytes,
        })
    }

    pub(super) fn fd(&self) -> RawFd {
        self.fd
    }

    pub(super) fn mark_directory(&self, path: &Path) -> Result<(), String> {
        let display_path = path.display().to_string();
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| format!("fanotify_mark path contains interior NUL: {display_path}"))?;
        let result = unsafe {
            libc::fanotify_mark(
                self.fd,
                libc::FAN_MARK_ADD,
                libc::FAN_OPEN_PERM | libc::FAN_EVENT_ON_CHILD,
                libc::AT_FDCWD,
                path.as_ptr(),
            )
        };
        if result < 0 {
            return Err(format!(
                "fanotify_mark {display_path}: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub(super) fn unmark_directory(&self, path: &Path) -> Result<(), String> {
        let display_path = path.display().to_string();
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| format!("fanotify unmark path contains interior NUL: {display_path}"))?;
        let result = unsafe {
            libc::fanotify_mark(
                self.fd,
                libc::FAN_MARK_REMOVE,
                libc::FAN_OPEN_PERM | libc::FAN_EVENT_ON_CHILD,
                libc::AT_FDCWD,
                path.as_ptr(),
            )
        };
        if result < 0 {
            return Err(format!(
                "fanotify unmark {display_path}: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub(super) fn ignore_path(&self, path: &Path, recursive: bool) -> Result<(), String> {
        let display_path = path.display().to_string();
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| format!("fanotify ignore path contains interior NUL: {display_path}"))?;
        let mask = if recursive {
            libc::FAN_OPEN_PERM | libc::FAN_EVENT_ON_CHILD
        } else {
            libc::FAN_OPEN_PERM
        };
        let result = unsafe {
            libc::fanotify_mark(
                self.fd,
                libc::FAN_MARK_ADD | libc::FAN_MARK_IGNORE_SURV,
                mask,
                libc::AT_FDCWD,
                path.as_ptr(),
            )
        };
        if result < 0 {
            return Err(format!(
                "fanotify ignore mark {display_path}: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub(super) fn drain<F>(&self, mut handle: F) -> Result<(), String>
    where
        F: FnMut(PermissionMetadata) -> Result<(), String>,
    {
        let mut buffer = vec![0_u8; self.event_buffer_bytes as usize];
        loop {
            let read_len = unsafe { libc::read(self.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read_len < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(());
                }
                return Err(format!("fanotify read: {error}"));
            }
            if read_len == 0 {
                return Ok(());
            }
            self.parse_buffer(&buffer[..read_len as usize], &mut handle)?;
        }
    }

    fn parse_buffer<F>(&self, buffer: &[u8], handle: &mut F) -> Result<(), String>
    where
        F: FnMut(PermissionMetadata) -> Result<(), String>,
    {
        let metadata_size = size_of::<libc::fanotify_event_metadata>();
        let mut offset = 0;
        while buffer.len().saturating_sub(offset) >= metadata_size {
            let metadata = unsafe {
                std::ptr::read_unaligned(
                    buffer[offset..]
                        .as_ptr()
                        .cast::<libc::fanotify_event_metadata>(),
                )
            };
            if metadata.vers != libc::FANOTIFY_METADATA_VERSION {
                return Err(format!(
                    "fanotify metadata version {} does not match expected {}",
                    metadata.vers,
                    libc::FANOTIFY_METADATA_VERSION
                ));
            }
            let event_len = metadata.event_len as usize;
            if event_len < metadata_size || offset + event_len > buffer.len() {
                return Err(format!(
                    "fanotify invalid event length {}",
                    metadata.event_len
                ));
            }
            if metadata.fd != libc::FAN_NOFD {
                handle(PermissionMetadata {
                    fd: metadata.fd,
                    pid: metadata.pid,
                    mask: metadata.mask,
                })?;
            }
            offset += event_len;
        }
        Ok(())
    }
}

impl Drop for FanotifyHandle {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

pub(super) struct PermissionEventFd {
    fd: RawFd,
}

impl PermissionEventFd {
    pub(super) fn new(fd: RawFd) -> Self {
        Self { fd }
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.fd
    }

    pub(super) fn file_key(&self) -> Result<FileKey, String> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe { libc::fstat(self.fd, stat.as_mut_ptr()) };
        if result < 0 {
            return Err(format!(
                "fanotify event fstat: {}",
                io::Error::last_os_error()
            ));
        }
        let stat = unsafe { stat.assume_init() };
        Ok(FileKey {
            dev: stat.st_dev,
            ino: stat.st_ino,
        })
    }

    pub(super) fn display_path(&self) -> Option<String> {
        std::fs::read_link(format!("/proc/self/fd/{}", self.fd))
            .ok()
            .map(|path| path.display().to_string())
    }
}

impl Drop for PermissionEventFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

pub(super) fn respond(fanotify_fd: RawFd, event_fd: RawFd, allow: bool) -> Result<(), String> {
    let response = libc::fanotify_response {
        fd: event_fd,
        response: if allow {
            libc::FAN_ALLOW
        } else {
            libc::FAN_DENY
        },
    };
    let written = unsafe {
        libc::write(
            fanotify_fd,
            (&response as *const libc::fanotify_response).cast(),
            size_of::<libc::fanotify_response>(),
        )
    };
    if written < 0 {
        return Err(format!(
            "fanotify response write: {}",
            io::Error::last_os_error()
        ));
    }
    if written as usize != size_of::<libc::fanotify_response>() {
        return Err(format!("fanotify response short write: {written}"));
    }
    Ok(())
}

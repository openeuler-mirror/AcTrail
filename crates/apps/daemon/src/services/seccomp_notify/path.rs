use std::collections::VecDeque;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Resolves each notification in its task's filesystem view. No path result is cached.
pub(crate) struct TraceePath {
    pid: u32,
    root: File,
    root_path: PathBuf,
}

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

impl TraceePath {
    pub(crate) fn new(pid: u32) -> io::Result<Self> {
        let source = PathBuf::from(format!("/proc/{pid}/root"));
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(&source)?;
        let root_path = std::fs::read_link(&source)?;
        Ok(Self {
            pid,
            root,
            root_path,
        })
    }

    pub(crate) fn executable(
        &self,
        requested: &str,
        dirfd: Option<i32>,
        empty_path: bool,
    ) -> io::Result<PathBuf> {
        if empty_path {
            return self.proc_link(&format!(
                "fd/{}",
                dirfd.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "empty executable path requires dirfd",
                    )
                })?
            ));
        }
        if let Some(path) = self.outside_relative(requested, dirfd, false)? {
            return Ok(path);
        }
        self.resolve(&self.absolute(requested, dirfd)?)
    }

    pub(crate) fn directory_entry(
        &self,
        requested: &str,
        dirfd: Option<i32>,
    ) -> io::Result<PathBuf> {
        if let Some(path) = self.outside_relative(requested, dirfd, true)? {
            return Ok(path);
        }
        let absolute = self.absolute(requested, dirfd)?;
        // mkdir/rmdir act on the final directory entry without following its link.
        match (absolute.parent(), absolute.file_name()) {
            (Some(parent), Some(name)) => Ok(self.resolve(parent)?.join(name)),
            _ => self.resolve(&absolute),
        }
    }

    fn absolute(&self, requested: &str, dirfd: Option<i32>) -> io::Result<PathBuf> {
        if requested.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path"));
        }
        let requested = Path::new(requested);
        if requested.is_absolute() {
            return Ok(requested.to_path_buf());
        }
        let base = match dirfd {
            None | Some(libc::AT_FDCWD) => self.proc_link("cwd")?,
            Some(fd) => self.proc_link(&format!("fd/{fd}"))?,
        };
        Ok(base.join(requested))
    }

    fn outside_relative(
        &self,
        requested: &str,
        dirfd: Option<i32>,
        directory_entry: bool,
    ) -> io::Result<Option<PathBuf>> {
        let requested = Path::new(requested);
        if requested.is_absolute()
            || requested.as_os_str().is_empty()
            || self.root_path == Path::new("/")
        {
            return Ok(None);
        }
        let source = match dirfd {
            None | Some(libc::AT_FDCWD) => PathBuf::from(format!("/proc/{}/cwd", self.pid)),
            Some(fd) => PathBuf::from(format!("/proc/{}/fd/{fd}", self.pid)),
        };
        if std::fs::read_link(&source)?.starts_with(&self.root_path) {
            return Ok(None);
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(source)?;
        let (lookup, leaf) = if directory_entry {
            match (requested.parent(), requested.file_name()) {
                (Some(parent), Some(name)) => (parent, Some(name)),
                _ => (requested, None),
            }
        } else {
            (requested, None)
        };
        let resolved = self.walk_from(directory, lookup)?;
        Ok(Some(match leaf {
            Some(name) => resolved.join(name),
            None => resolved,
        }))
    }

    /// A cwd or directory fd can survive chroot outside the new root. Walk from
    /// that actual descriptor; absolute symlinks still restart in the task root.
    fn walk_from(&self, mut directory: File, path: &Path) -> io::Result<PathBuf> {
        let mut pending = path
            .components()
            .map(|part| part.as_os_str().to_owned())
            .collect::<VecDeque<_>>();
        // Linux namei permits at most 40 followed links per pathname lookup.
        let mut remaining_links = 40_u32;
        while let Some(part) = pending.pop_front() {
            if part == "/" {
                directory = self.root.try_clone()?;
                continue;
            }
            if part == "." || (part == ".." && self.is_root(&directory)?) {
                continue;
            }
            let parent_path =
                std::fs::read_link(format!("/proc/self/fd/{}", directory.as_raw_fd()))?;
            let candidate = self.in_root(parent_path).join(&part);
            let next = if (candidate == Path::new("/proc/self")
                || candidate == Path::new("/proc/thread-self"))
                && Self::is_procfs(directory.as_raw_fd())?
            {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
                    .open(self.proc_task_path(candidate == Path::new("/proc/thread-self"))?)?
            } else {
                Self::open_at(directory.as_raw_fd(), &part, true)?
            };
            if next.metadata()?.file_type().is_symlink() {
                if remaining_links == 0 {
                    return Err(io::Error::from_raw_os_error(libc::ELOOP));
                }
                remaining_links -= 1;
                if Self::is_proc_magic_link(&candidate) && Self::is_procfs(next.as_raw_fd())? {
                    directory = Self::open_at(directory.as_raw_fd(), &part, false)?;
                    continue;
                }
                let target = std::fs::read_link(
                    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(&part),
                )?;
                for step in target.components().rev() {
                    pending.push_front(step.as_os_str().to_owned());
                }
            } else {
                directory = next;
            }
        }
        Ok(self.in_root(std::fs::read_link(format!(
            "/proc/self/fd/{}",
            directory.as_raw_fd()
        ))?))
    }

    fn open_at(directory: RawFd, name: &std::ffi::OsStr, nofollow: bool) -> io::Result<File> {
        let name = CString::new(name.as_bytes())?;
        let flags = libc::O_PATH | libc::O_CLOEXEC | if nofollow { libc::O_NOFOLLOW } else { 0 };
        let fd = unsafe { libc::openat(directory, name.as_ptr(), flags) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    fn is_procfs(fd: RawFd) -> io::Result<bool> {
        let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        if unsafe { libc::fstatfs(fd, filesystem.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { filesystem.assume_init() }.f_type == libc::PROC_SUPER_MAGIC)
    }

    fn is_root(&self, directory: &File) -> io::Result<bool> {
        let root = Self::path_identity(self.root.as_raw_fd())?;
        Ok(Self::path_identity(directory.as_raw_fd())? == root)
    }

    fn path_identity(fd: RawFd) -> io::Result<(u64, u64, u32, u32)> {
        let mut metadata = std::mem::MaybeUninit::<libc::statx>::zeroed();
        let result = unsafe {
            libc::statx(
                fd,
                c"".as_ptr(),
                libc::AT_EMPTY_PATH | libc::AT_STATX_DONT_SYNC,
                libc::STATX_INO | libc::STATX_MNT_ID,
                metadata.as_mut_ptr(),
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        let metadata = unsafe { metadata.assume_init() };
        Ok((
            metadata.stx_mnt_id,
            metadata.stx_ino,
            metadata.stx_dev_major,
            metadata.stx_dev_minor,
        ))
    }

    fn proc_link(&self, suffix: &str) -> io::Result<PathBuf> {
        Ok(self.in_root(std::fs::read_link(format!("/proc/{}/{suffix}", self.pid))?))
    }

    fn in_root(&self, host_path: PathBuf) -> PathBuf {
        // An open descriptor can still name an unlinked file or an object outside
        // a later chroot. Keep the kernel's name when no root-relative name exists.
        host_path
            .strip_prefix(&self.root_path)
            .map(|relative| Path::new("/").join(relative))
            .unwrap_or(host_path)
    }

    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        let raw = CString::new(path.as_os_str().as_bytes())?;
        let how = OpenHow {
            flags: (libc::O_PATH | libc::O_CLOEXEC) as u64,
            mode: 0,
            // Linux openat2 RESOLVE_IN_ROOT interprets absolute links in the task root.
            resolve: 0x10,
        };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                self.root.as_raw_fd(),
                raw.as_ptr(),
                &how as *const OpenHow,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if fd >= 0 {
            let file = unsafe { File::from_raw_fd(fd as i32) };
            return Ok(self.in_root(std::fs::read_link(format!(
                "/proc/self/fd/{}",
                file.as_raw_fd()
            ))?));
        }
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::EXDEV)) {
            // IN_ROOT cannot traverse procfs magic links. Resolve those in the
            // notification task, including links reached through ordinary aliases.
            return self.walk_from(self.root.try_clone()?, path);
        }
        Err(error)
    }

    fn proc_task_path(&self, thread: bool) -> io::Result<PathBuf> {
        if thread {
            return Ok(PathBuf::from(format!("/proc/{}", self.pid)));
        }
        // seccomp_notif.pid identifies the notifying thread. proc/self uses
        // its thread group, which can have a different fd table after unshare.
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.pid))?;
        let tgid = status
            .lines()
            .find_map(|line| line.strip_prefix("Tgid:"))
            .and_then(|value| value.trim().parse::<u32>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing task Tgid"))?;
        Ok(PathBuf::from(format!("/proc/{tgid}")))
    }

    fn is_proc_magic_link(path: &Path) -> bool {
        let Some(relative) = path.strip_prefix("/proc").ok() else {
            return false;
        };
        let mut parts = relative.components();
        let _task = parts.next();
        matches!(
            parts.next().and_then(|part| part.as_os_str().to_str()),
            Some("exe" | "cwd" | "root" | "fd")
        )
    }
}

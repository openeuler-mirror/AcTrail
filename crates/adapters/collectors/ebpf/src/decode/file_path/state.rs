//! Userspace state for raw file syscall events.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use model_core::ids::TraceId;

use crate::loader::KernelFilePathEvent;
use crate::procfs::{FdTargetKind, read_process_cwd, resolve_fd_observation};

use super::user_path::{UserPathRead, read_process_path};

pub(super) const FILE_FD_MISSING: u32 = u32::MAX;
pub(super) const PATH_FLAG_CAPTURED: u32 = 1;
pub(super) const PATH_FLAG_TRUNCATED: u32 = 2;
pub(super) const PATH_FLAG_FAULT: u32 = 4;

pub(super) const FILE_PHASE_ENTER: u32 = 1;
pub(super) const FILE_PHASE_EXIT: u32 = 2;

pub(super) const FILE_SYSCALL_OPEN: u32 = 1;
pub(super) const FILE_SYSCALL_OPENAT: u32 = 2;
pub(super) const FILE_SYSCALL_CREAT: u32 = 3;
pub(super) const FILE_SYSCALL_UNLINK: u32 = 4;
pub(super) const FILE_SYSCALL_UNLINKAT: u32 = 5;
pub(super) const FILE_SYSCALL_RENAME: u32 = 6;
pub(super) const FILE_SYSCALL_RENAMEAT: u32 = 7;
pub(super) const FILE_SYSCALL_RENAMEAT2: u32 = 8;
pub(super) const FILE_SYSCALL_MKDIR: u32 = 9;
pub(super) const FILE_SYSCALL_MKDIRAT: u32 = 10;
pub(super) const FILE_SYSCALL_RMDIR: u32 = 11;
pub(super) const FILE_SYSCALL_TRUNCATE: u32 = 12;
pub(super) const FILE_SYSCALL_FTRUNCATE: u32 = 13;
pub(super) const FILE_SYSCALL_MMAP: u32 = 14;
pub(super) const FILE_SYSCALL_CLOSE: u32 = 15;
pub(super) const FILE_SYSCALL_DUP: u32 = 16;
pub(super) const FILE_SYSCALL_DUP2: u32 = 17;
pub(super) const FILE_SYSCALL_DUP3: u32 = 18;
pub(super) const FILE_SYSCALL_FCNTL: u32 = 19;
pub(super) const FILE_SYSCALL_CHDIR: u32 = 20;
pub(super) const FILE_SYSCALL_FCHDIR: u32 = 21;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FileSyscallOutcome {
    pub enter: KernelFilePathEvent,
    pub result: i64,
    pub primary_path: PathResolution,
    pub secondary_path: Option<PathResolution>,
    pub fd_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PathResolution {
    pub raw: Option<String>,
    pub resolved: Option<String>,
    pub source: &'static str,
    pub userspace_retry: bool,
    pub userspace_retry_truncated: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FileTracker {
    pending: BTreeMap<PendingKey, KernelFilePathEvent>,
    pub(super) processes: BTreeMap<u32, ProcessFileState>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PendingKey {
    trace_id: TraceId,
    tid: u32,
    syscall: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ProcessFileState {
    pub(super) cwd: Option<String>,
    pub(super) fds: BTreeMap<u32, String>,
}

impl FileTracker {
    pub(crate) fn seed_process(&mut self, pid: u32) {
        self.ensure_process(pid);
    }

    pub(crate) fn inherit_process(&mut self, parent_pid: u32, child_pid: u32) {
        let inherited = self.ensure_process(parent_pid).clone();
        self.processes.insert(child_pid, inherited);
    }

    pub(crate) fn exec_process(&mut self, pid: u32) {
        let cwd = read_process_cwd(pid).or_else(|| self.processes.get(&pid)?.cwd.clone());
        self.processes.entry(pid).or_default().cwd = cwd;
    }

    pub(crate) fn remove_process(&mut self, pid: u32) {
        self.processes.remove(&pid);
        self.pending
            .retain(|key, event| key.tid != pid && event.pid != pid);
    }

    pub(crate) fn resolve_fd_path(&mut self, pid: u32, fd: u32) -> Option<String> {
        if fd == FILE_FD_MISSING {
            return None;
        }
        if let Some(path) = self
            .processes
            .get(&pid)
            .and_then(|state| state.fds.get(&fd))
            .cloned()
        {
            return Some(path);
        }
        let observation = resolve_fd_observation(pid, fd).ok().flatten()?;
        if observation.kind != FdTargetKind::RegularFile {
            return None;
        }
        self.ensure_process(pid)
            .fds
            .insert(fd, observation.target.clone());
        Some(observation.target)
    }

    pub(super) fn record(&mut self, event: KernelFilePathEvent) -> Option<FileSyscallOutcome> {
        match event.phase {
            FILE_PHASE_ENTER => {
                self.pending.insert(pending_key(&event), event);
                None
            }
            FILE_PHASE_EXIT => self.complete(event),
            _ => None,
        }
    }

    fn complete(&mut self, exit: KernelFilePathEvent) -> Option<FileSyscallOutcome> {
        let enter = self.pending.remove(&pending_key(&exit))?;
        let primary_path = self.resolve_primary_path(&enter);
        let secondary_path = self.resolve_secondary_path(&enter);
        let fd_path =
            self.apply_successful_exit(&enter, exit.result, &primary_path, &secondary_path);
        Some(FileSyscallOutcome {
            enter,
            result: exit.result,
            primary_path,
            secondary_path,
            fd_path,
        })
    }

    fn apply_successful_exit(
        &mut self,
        enter: &KernelFilePathEvent,
        result: i64,
        primary_path: &PathResolution,
        secondary_path: &Option<PathResolution>,
    ) -> Option<String> {
        if result < 0 {
            return None;
        }
        match enter.aux {
            FILE_SYSCALL_OPEN | FILE_SYSCALL_OPENAT | FILE_SYSCALL_CREAT => {
                let fd = u32::try_from(result).ok()?;
                let path = primary_path
                    .resolved
                    .clone()
                    .or_else(|| primary_path.raw.clone())?;
                self.ensure_process(enter.pid).fds.insert(fd, path.clone());
                Some(path)
            }
            FILE_SYSCALL_CLOSE => {
                let path = self.resolve_fd_path(enter.pid, enter.arg0 as u32);
                self.ensure_process(enter.pid)
                    .fds
                    .remove(&(enter.arg0 as u32));
                path
            }
            FILE_SYSCALL_DUP | FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 | FILE_SYSCALL_FCNTL => {
                self.apply_dup_like_exit(enter, result);
                None
            }
            FILE_SYSCALL_CHDIR => {
                if let Some(path) = primary_path
                    .resolved
                    .clone()
                    .or_else(|| primary_path.raw.clone())
                {
                    self.ensure_process(enter.pid).cwd = Some(path);
                }
                None
            }
            FILE_SYSCALL_FCHDIR => {
                if let Some(path) = self.resolve_fd_path(enter.pid, enter.arg0 as u32) {
                    self.ensure_process(enter.pid).cwd = Some(path);
                }
                None
            }
            FILE_SYSCALL_RENAME | FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => {
                self.apply_rename(primary_path, secondary_path);
                None
            }
            FILE_SYSCALL_MMAP | FILE_SYSCALL_FTRUNCATE => self.resolve_fd_path(enter.pid, enter.fd),
            _ => None,
        }
    }

    fn apply_dup_like_exit(&mut self, enter: &KernelFilePathEvent, result: i64) {
        let Some(source) = self.resolve_fd_path(enter.pid, enter.arg0 as u32) else {
            return;
        };
        let Some(target_fd) = dup_target_fd(enter, result) else {
            return;
        };
        self.ensure_process(enter.pid).fds.insert(target_fd, source);
    }

    fn apply_rename(
        &mut self,
        primary_path: &PathResolution,
        secondary_path: &Option<PathResolution>,
    ) {
        let Some(source) = primary_path.resolved.as_ref().or(primary_path.raw.as_ref()) else {
            return;
        };
        let Some(target) = secondary_path
            .as_ref()
            .and_then(|path| path.resolved.as_ref().or(path.raw.as_ref()))
        else {
            return;
        };
        for state in self.processes.values_mut() {
            for fd_path in state.fds.values_mut() {
                if fd_path == source {
                    *fd_path = target.clone();
                }
            }
        }
    }

    fn resolve_primary_path(&mut self, event: &KernelFilePathEvent) -> PathResolution {
        let retry = retry_primary_path(event);
        let raw = path_string(&event.path, event.path_flags)
            .or_else(|| retry.as_ref().map(|path_read| path_read.value.clone()));
        let dirfd = primary_dirfd(event);
        self.resolve_path(event.pid, raw, dirfd, retry.as_ref())
    }

    fn resolve_secondary_path(&mut self, event: &KernelFilePathEvent) -> Option<PathResolution> {
        let retry = retry_secondary_path(event);
        let raw = path_string(&event.secondary_path, event.secondary_path_flags)
            .or_else(|| retry.as_ref().map(|path_read| path_read.value.clone()))?;
        let dirfd = secondary_dirfd(event);
        Some(self.resolve_path(event.pid, Some(raw), dirfd, retry.as_ref()))
    }

    fn resolve_path(
        &mut self,
        pid: u32,
        raw: Option<String>,
        dirfd: Option<u32>,
        retry: Option<&UserPathRead>,
    ) -> PathResolution {
        let userspace_retry = retry.is_some();
        let userspace_retry_truncated = retry.is_some_and(|path_read| path_read.truncated);
        let Some(raw_path) = raw else {
            return PathResolution {
                raw: None,
                resolved: None,
                source: "missing",
                userspace_retry,
                userspace_retry_truncated,
            };
        };
        if Path::new(&raw_path).is_absolute() {
            return PathResolution {
                raw: Some(raw_path.clone()),
                resolved: Some(raw_path),
                source: "absolute",
                userspace_retry,
                userspace_retry_truncated,
            };
        }
        let base = match dirfd {
            Some(fd) if fd as i32 != libc::AT_FDCWD => self.resolve_fd_path(pid, fd),
            _ => self.ensure_process(pid).cwd.clone(),
        };
        let Some(base) = base else {
            return PathResolution {
                raw: Some(raw_path),
                resolved: None,
                source: "unresolved_relative",
                userspace_retry,
                userspace_retry_truncated,
            };
        };
        PathResolution {
            resolved: Some(PathBuf::from(base).join(&raw_path).display().to_string()),
            raw: Some(raw_path),
            source: if dirfd.is_some_and(|fd| fd as i32 != libc::AT_FDCWD) {
                "dirfd"
            } else {
                "cwd"
            },
            userspace_retry,
            userspace_retry_truncated,
        }
    }

    fn ensure_process(&mut self, pid: u32) -> &mut ProcessFileState {
        self.processes
            .entry(pid)
            .or_insert_with(|| ProcessFileState {
                cwd: read_process_cwd(pid),
                fds: BTreeMap::new(),
            })
    }
}

fn pending_key(event: &KernelFilePathEvent) -> PendingKey {
    PendingKey {
        trace_id: event.trace_id,
        tid: event.tid,
        syscall: event.aux,
    }
}

fn path_string(bytes: &[u8], flags: u32) -> Option<String> {
    if flags & PATH_FLAG_CAPTURED == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn retry_primary_path(event: &KernelFilePathEvent) -> Option<UserPathRead> {
    if event.path_flags & PATH_FLAG_FAULT == 0 {
        return None;
    }
    read_process_path(
        event.pid,
        primary_path_pointer(event)?,
        event.path_max_bytes,
    )
}

fn retry_secondary_path(event: &KernelFilePathEvent) -> Option<UserPathRead> {
    if event.secondary_path_flags & PATH_FLAG_FAULT == 0 {
        return None;
    }
    read_process_path(
        event.pid,
        secondary_path_pointer(event)?,
        event.path_max_bytes,
    )
}

fn primary_path_pointer(event: &KernelFilePathEvent) -> Option<u64> {
    match event.aux {
        FILE_SYSCALL_OPEN
        | FILE_SYSCALL_CREAT
        | FILE_SYSCALL_UNLINK
        | FILE_SYSCALL_RENAME
        | FILE_SYSCALL_MKDIR
        | FILE_SYSCALL_RMDIR
        | FILE_SYSCALL_TRUNCATE
        | FILE_SYSCALL_CHDIR => Some(event.arg0),
        FILE_SYSCALL_OPENAT
        | FILE_SYSCALL_UNLINKAT
        | FILE_SYSCALL_RENAMEAT
        | FILE_SYSCALL_RENAMEAT2
        | FILE_SYSCALL_MKDIRAT => Some(event.arg1),
        _ => None,
    }
}

fn secondary_path_pointer(event: &KernelFilePathEvent) -> Option<u64> {
    match event.aux {
        FILE_SYSCALL_RENAME => Some(event.arg1),
        FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => Some(event.arg3),
        _ => None,
    }
}

fn primary_dirfd(event: &KernelFilePathEvent) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_OPENAT
        | FILE_SYSCALL_UNLINKAT
        | FILE_SYSCALL_RENAMEAT
        | FILE_SYSCALL_RENAMEAT2
        | FILE_SYSCALL_MKDIRAT => Some(event.arg0 as u32),
        _ => None,
    }
}

fn secondary_dirfd(event: &KernelFilePathEvent) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => Some(event.arg2 as u32),
        _ => None,
    }
}

fn dup_target_fd(event: &KernelFilePathEvent, result: i64) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_DUP | FILE_SYSCALL_FCNTL => u32::try_from(result).ok(),
        FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 => Some(event.arg1 as u32),
        _ => None,
    }
}

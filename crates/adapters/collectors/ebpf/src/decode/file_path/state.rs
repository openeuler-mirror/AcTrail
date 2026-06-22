//! Userspace state for raw file syscall events.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use crate::loader::KernelFilePathEvent;

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
pub(super) const FILE_SYSCALL_OPENAT2: u32 = 22;

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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FileTracker {
    pending: BTreeMap<PendingKey, KernelFilePathEvent>,
    pending_processes: BTreeMap<PendingKey, ProcessFileKey>,
    pub(super) processes: BTreeMap<ProcessFileKey, ProcessFileState>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PendingKey {
    trace_id: TraceId,
    tid: u32,
    syscall: u32,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ProcessFileKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ProcessFileState {
    pub(super) cwd: Option<String>,
    pub(super) fds: BTreeMap<u32, String>,
}

impl FileTracker {
    pub(crate) fn seed_process(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        cwd: Option<String>,
    ) {
        let key = ProcessFileKey { trace_id, process };
        let state = self.processes.entry(key).or_default();
        if let Some(cwd) = cwd.and_then(|path| absolute_path(&path)) {
            state.cwd = Some(cwd);
        }
    }

    pub(crate) fn inherit_process(
        &mut self,
        trace_id: TraceId,
        parent: &ProcessIdentity,
        child: ProcessIdentity,
    ) {
        let parent_key = ProcessFileKey {
            trace_id,
            process: parent.clone(),
        };
        let child_key = ProcessFileKey {
            trace_id,
            process: child,
        };
        let inherited = self.processes.get(&parent_key).cloned().unwrap_or_default();
        self.processes.insert(child_key, inherited);
    }

    pub(crate) fn exec_process(&mut self, trace_id: TraceId, process: ProcessIdentity) {
        let key = ProcessFileKey { trace_id, process };
        self.processes.entry(key).or_default();
    }

    pub(crate) fn remove_trace(&mut self, trace_id: TraceId) {
        self.processes.retain(|key, _| key.trace_id != trace_id);
        self.pending.retain(|key, _| key.trace_id != trace_id);
        self.pending_processes
            .retain(|key, _| key.trace_id != trace_id);
    }

    pub(crate) fn resolve_fd_path(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        fd: u32,
    ) -> Option<String> {
        if fd == FILE_FD_MISSING {
            return None;
        }
        let key = ProcessFileKey {
            trace_id,
            process: process.clone(),
        };
        self.processes
            .get(&key)
            .and_then(|state| state.fds.get(&fd))
            .cloned()
    }

    pub(super) fn record(
        &mut self,
        event: KernelFilePathEvent,
        process: ProcessIdentity,
    ) -> Option<FileSyscallOutcome> {
        match event.phase {
            FILE_PHASE_ENTER => {
                let key = pending_key(&event);
                self.pending_processes.insert(
                    key.clone(),
                    ProcessFileKey {
                        trace_id: event.trace_id,
                        process,
                    },
                );
                self.pending.insert(key, event);
                None
            }
            FILE_PHASE_EXIT => self.complete(event),
            _ => None,
        }
    }

    fn complete(&mut self, exit: KernelFilePathEvent) -> Option<FileSyscallOutcome> {
        let key = pending_key(&exit);
        let enter = self.pending.remove(&key)?;
        let process_key = self.pending_processes.remove(&key)?;
        let primary_path = self.resolve_primary_path(&process_key, &enter);
        let secondary_path = self.resolve_secondary_path(&process_key, &enter);
        let fd_path = self.apply_successful_exit(
            &process_key,
            &enter,
            exit.result,
            &primary_path,
            &secondary_path,
        );
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
        process_key: &ProcessFileKey,
        enter: &KernelFilePathEvent,
        result: i64,
        primary_path: &PathResolution,
        secondary_path: &Option<PathResolution>,
    ) -> Option<String> {
        if result < 0 {
            return None;
        }
        match enter.aux {
            FILE_SYSCALL_OPEN | FILE_SYSCALL_OPENAT | FILE_SYSCALL_CREAT | FILE_SYSCALL_OPENAT2 => {
                let fd = u32::try_from(result).ok()?;
                let path = resolved_absolute_path(primary_path)?;
                self.ensure_process(process_key)
                    .fds
                    .insert(fd, path.clone());
                Some(path)
            }
            FILE_SYSCALL_CLOSE => {
                let path = self.resolve_fd_path(
                    process_key.trace_id,
                    &process_key.process,
                    enter.arg0 as u32,
                );
                self.ensure_process(process_key)
                    .fds
                    .remove(&(enter.arg0 as u32));
                path
            }
            FILE_SYSCALL_DUP | FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 => {
                self.apply_dup_like_exit(process_key, enter, result);
                None
            }
            FILE_SYSCALL_FCNTL if fcntl_duplicates_fd(enter) => {
                self.apply_dup_like_exit(process_key, enter, result);
                None
            }
            FILE_SYSCALL_CHDIR => {
                if let Some(path) = resolved_absolute_path(primary_path) {
                    self.ensure_process(process_key).cwd = Some(path);
                }
                None
            }
            FILE_SYSCALL_FCHDIR => {
                if let Some(path) = self.resolve_fd_path(
                    process_key.trace_id,
                    &process_key.process,
                    enter.arg0 as u32,
                ) {
                    self.ensure_process(process_key).cwd = Some(path);
                }
                None
            }
            FILE_SYSCALL_RENAME | FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => {
                self.apply_rename(primary_path, secondary_path);
                None
            }
            FILE_SYSCALL_MMAP | FILE_SYSCALL_FTRUNCATE => {
                self.resolve_fd_path(process_key.trace_id, &process_key.process, enter.fd)
            }
            _ => None,
        }
    }

    fn apply_dup_like_exit(
        &mut self,
        process_key: &ProcessFileKey,
        enter: &KernelFilePathEvent,
        result: i64,
    ) {
        let Some(source) = self.resolve_fd_path(
            process_key.trace_id,
            &process_key.process,
            enter.arg0 as u32,
        ) else {
            return;
        };
        let Some(target_fd) = dup_target_fd(enter, result) else {
            return;
        };
        self.ensure_process(process_key)
            .fds
            .insert(target_fd, source);
    }

    fn apply_rename(
        &mut self,
        primary_path: &PathResolution,
        secondary_path: &Option<PathResolution>,
    ) {
        let Some(source) = resolved_absolute_path(primary_path) else {
            return;
        };
        let Some(target) = secondary_path.as_ref().and_then(resolved_absolute_path) else {
            return;
        };
        for state in self.processes.values_mut() {
            for fd_path in state.fds.values_mut() {
                if fd_path.as_str() == source.as_str() {
                    *fd_path = target.clone();
                }
            }
        }
    }

    fn resolve_primary_path(
        &mut self,
        process_key: &ProcessFileKey,
        event: &KernelFilePathEvent,
    ) -> PathResolution {
        let raw = path_string(&event.path, event.path_flags);
        let dirfd = primary_dirfd(event);
        self.resolve_path(process_key, raw, dirfd)
    }

    fn resolve_secondary_path(
        &mut self,
        process_key: &ProcessFileKey,
        event: &KernelFilePathEvent,
    ) -> Option<PathResolution> {
        let raw = path_string(&event.secondary_path, event.secondary_path_flags)?;
        let dirfd = secondary_dirfd(event);
        Some(self.resolve_path(process_key, Some(raw), dirfd))
    }

    fn resolve_path(
        &mut self,
        process_key: &ProcessFileKey,
        raw: Option<String>,
        dirfd: Option<u32>,
    ) -> PathResolution {
        let Some(raw_path) = raw else {
            return PathResolution {
                raw: None,
                resolved: None,
                source: "missing",
            };
        };
        if Path::new(&raw_path).is_absolute() {
            let resolved = lexically_normalize_path(&raw_path);
            return PathResolution {
                raw: Some(raw_path.clone()),
                resolved: Some(resolved),
                source: "absolute",
            };
        }
        let base = match dirfd {
            Some(fd) if fd as i32 != libc::AT_FDCWD => {
                self.resolve_fd_path(process_key.trace_id, &process_key.process, fd)
            }
            _ => self.ensure_process(process_key).cwd.clone(),
        };
        let Some(base) = base.and_then(|path| absolute_path(&path)) else {
            return PathResolution {
                raw: Some(raw_path),
                resolved: None,
                source: "unresolved_relative",
            };
        };
        let resolved =
            lexically_normalize_path(&PathBuf::from(base).join(&raw_path).display().to_string());
        PathResolution {
            resolved: Some(resolved),
            raw: Some(raw_path),
            source: if dirfd.is_some_and(|fd| fd as i32 != libc::AT_FDCWD) {
                "dirfd"
            } else {
                "cwd"
            },
        }
    }

    fn ensure_process(&mut self, process_key: &ProcessFileKey) -> &mut ProcessFileState {
        self.processes.entry(process_key.clone()).or_default()
    }
}

fn lexically_normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if absolute {
                    let _ = parts.pop();
                } else if parts.last().is_some_and(|last| *last != "..") {
                    let _ = parts.pop();
                } else {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }
    if absolute {
        if parts.is_empty() {
            return "/".to_string();
        }
        return format!("/{}", parts.join("/"));
    }
    if parts.is_empty() {
        return ".".to_string();
    }
    parts.join("/")
}

fn resolved_absolute_path(path: &PathResolution) -> Option<String> {
    path.resolved.as_deref().and_then(absolute_path)
}

fn absolute_path(path: &str) -> Option<String> {
    if !Path::new(path).is_absolute() {
        return None;
    }
    Some(lexically_normalize_path(path))
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

fn primary_dirfd(event: &KernelFilePathEvent) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_OPENAT
        | FILE_SYSCALL_OPENAT2
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

pub(super) fn dup_target_fd(event: &KernelFilePathEvent, result: i64) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_DUP => u32::try_from(result).ok(),
        FILE_SYSCALL_FCNTL if fcntl_duplicates_fd(event) => u32::try_from(result).ok(),
        FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 => Some(event.arg1 as u32),
        _ => None,
    }
}

pub(super) fn fcntl_duplicates_fd(event: &KernelFilePathEvent) -> bool {
    i32::try_from(event.arg1)
        .is_ok_and(|command| matches!(command, libc::F_DUPFD | libc::F_DUPFD_CLOEXEC))
}

//! Userspace state for raw file syscall events.

mod completion;
mod mcp_context;
mod path_utils;
mod syscall;

use path_utils::*;
pub(super) use syscall::{dup_target_fd, fcntl_duplicates_fd};
use syscall::{
    duplicated_fd_close_on_exec, fcntl_sets_fd_flags, ipc_kind_from_event, ipc_pair_close_on_exec,
    open_requests_creation, path_string, pending_key, primary_dirfd, secondary_dirfd,
};

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::rc::Rc;

use crate::maps::BindingStateMap;
use config_core::daemon::{FileCollectionConfig, IpcLineageConfig};
use model_core::capability::Capability;
use model_core::ids::TraceId;
use model_core::process::ProcessObservation;

use crate::loader::KernelFilePathEvent;

use super::mcp_stdio::McpStdioTracker;
use crate::decode::FdIpcKind;

pub(super) const FILE_FD_MISSING: u32 = u32::MAX;
pub(super) const PATH_FLAG_CAPTURED: u32 = 1;
pub(super) const PATH_FLAG_TRUNCATED: u32 = 2;
pub(super) const PATH_FLAG_FAULT: u32 = 4;
pub(super) const PATH_FLAG_DIRECTORY: u32 = 1 << 8;
pub(super) const PATH_FLAG_INTERNAL_CONTEXT: u32 = 1 << 9;

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
pub(super) const FILE_SYSCALL_PIPE: u32 = 23;
pub(super) const FILE_SYSCALL_PIPE2: u32 = 24;
pub(super) const FILE_SYSCALL_SOCKETPAIR: u32 = 25;
pub(super) const FILE_SYSCALL_CLOSE_RANGE: u32 = 27;
pub(super) const FILE_SYSCALL_IOCTL_CLOEXEC: u32 = 28;
pub(super) const FILE_SYSCALL_IOCTL_NCLOEXEC: u32 = 29;

pub(super) const FILE_IPC_KIND_PIPE: u64 = 1;
pub(super) const FILE_IPC_KIND_UNIX_SOCKET: u64 = 2;
const CLOSE_RANGE_CLOEXEC: u64 = 1 << 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FileSyscallOutcome {
    pub syscall: KernelFilePathEvent,
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

/// Consumers of userspace file/FD context for one trace, derived from bindings.
#[derive(Clone, Copy)]
pub(crate) struct FileContextConsumers {
    pub(crate) file_paths: bool,
    pub(crate) mcp_stdio: bool,
}

impl FileContextConsumers {
    pub(crate) fn any(self) -> bool {
        self.file_paths || self.mcp_stdio
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileTracker {
    pending: BTreeMap<PendingKey, PendingFileOperation>,
    pub(super) processes: BTreeMap<ProcessFileKey, ProcessFileState>,
    mcp_stdio: McpStdioTracker,
    process_rcs: RefCell<HashMap<ProcessObservation, Rc<ProcessObservation>>>,
    file_capture_enabled: bool,
    file_collection: FileCollectionConfig,
}

impl Default for FileTracker {
    fn default() -> Self {
        Self::new(IpcLineageConfig::default(), true)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PendingKey {
    trace_id: TraceId,
    tid: u32,
    syscall: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingFileOperation {
    syscall: KernelFilePathEvent,
    process: ProcessFileKey,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ProcessFileKey {
    pub(super) trace_id: TraceId,
    pub(super) process: Rc<ProcessObservation>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ProcessFileState {
    pub(super) cwd: Option<String>,
    pub(super) fds: BTreeMap<u32, String>,
    creation_requested_fds: BTreeSet<u32>,
}

pub(crate) struct ResolvedFileDescriptor {
    pub(crate) path: String,
    pub(crate) creation_requested: bool,
}

impl FileTracker {
    pub(crate) fn new(ipc_lineage: IpcLineageConfig, mcp_projection_enabled: bool) -> Self {
        Self {
            pending: BTreeMap::new(),
            processes: BTreeMap::new(),
            mcp_stdio: McpStdioTracker::new(ipc_lineage, mcp_projection_enabled),
            process_rcs: RefCell::new(HashMap::new()),
            file_capture_enabled: true,
            file_collection: FileCollectionConfig::default(),
        }
    }

    pub(crate) fn context_consumers(
        &self,
        trace_id: TraceId,
        bindings: &BindingStateMap,
    ) -> FileContextConsumers {
        FileContextConsumers {
            file_paths: self.file_capture_enabled
                && (bindings.trace_has_capability(trace_id, &Capability::FsAccessBasic)
                    || bindings.trace_has_capability(trace_id, &Capability::FsMmap)),
            mcp_stdio: self.mcp_stdio_enabled()
                && bindings.trace_has_capability(trace_id, &Capability::StdioChunk),
        }
    }

    pub(crate) fn with_file_capture(mut self, enabled: bool) -> Self {
        self.file_capture_enabled = enabled;
        self
    }

    pub(crate) fn with_file_collection(mut self, collection: FileCollectionConfig) -> Self {
        self.file_collection = collection;
        self
    }

    pub(in crate::decode) fn file_capture_enabled(&self) -> bool {
        self.file_capture_enabled
    }

    /// Intern the process observation so repeated `ProcessFileKey`
    /// constructions (per fd/path lookup and per `ensure_process` entry) only
    /// bump an `Rc` refcount instead of cloning the embedded namespace string.
    fn intern_process(&self, process: &ProcessObservation) -> Rc<ProcessObservation> {
        let mut rcs = self.process_rcs.borrow_mut();
        if let Some(rc) = rcs.get(process) {
            return Rc::clone(rc);
        }
        let rc = Rc::new(process.clone());
        rcs.insert(process.clone(), Rc::clone(&rc));
        rc
    }

    pub(crate) fn seed_process(
        &mut self,
        trace_id: TraceId,
        process: ProcessObservation,
        cwd: Option<String>,
        consumers: FileContextConsumers,
    ) {
        if !consumers.any() {
            return;
        }
        let key = ProcessFileKey {
            trace_id,
            process: self.intern_process(&process),
        };
        if consumers.file_paths {
            let state = self.processes.entry(key.clone()).or_default();
            if let Some(cwd) = cwd.and_then(|path| absolute_path(&path)) {
                state.cwd = Some(cwd);
            }
        }
        if consumers.mcp_stdio {
            self.mcp_stdio.seed_process(key);
        }
    }

    pub(crate) fn inherit_process(
        &mut self,
        trace_id: TraceId,
        parent: &ProcessObservation,
        child: ProcessObservation,
        consumers: FileContextConsumers,
    ) {
        if !consumers.any() {
            return;
        }
        let parent_key = ProcessFileKey {
            trace_id,
            process: self.intern_process(parent),
        };
        let child_key = ProcessFileKey {
            trace_id,
            process: self.intern_process(&child),
        };
        if consumers.file_paths {
            let inherited = self.processes.get(&parent_key).cloned().unwrap_or_default();
            self.processes.insert(child_key.clone(), inherited);
        }
        if consumers.mcp_stdio {
            self.mcp_stdio.inherit_process(&parent_key, child_key);
        }
    }

    pub(crate) fn exec_process(
        &mut self,
        trace_id: TraceId,
        process: ProcessObservation,
        observed_ktime_ns: u64,
        consumers: FileContextConsumers,
    ) {
        if !consumers.any() {
            return;
        }
        let key = ProcessFileKey {
            trace_id,
            process: self.intern_process(&process),
        };
        if !consumers.file_paths || !self.processes.contains_key(&key) {
            let provisional = process.host.clone().map(|host| ProcessFileKey {
                trace_id,
                process: self.intern_process(&ProcessObservation::host(host)),
            });
            let state = consumers.file_paths.then(|| {
                provisional
                    .as_ref()
                    .and_then(|provisional| self.processes.remove(provisional))
                    .unwrap_or_default()
            });
            if consumers.mcp_stdio
                && let Some(provisional) = provisional.filter(|provisional| provisional != &key)
            {
                self.mcp_stdio.rekey_process(&provisional, key.clone());
            }
            if let Some(state) = state {
                self.processes.insert(key.clone(), state);
            }
        }
        if consumers.mcp_stdio {
            self.mcp_stdio.exec_process(key, observed_ktime_ns);
        }
    }

    pub(crate) fn exit_process(
        &mut self,
        trace_id: TraceId,
        process: ProcessObservation,
        observed_ktime_ns: u64,
        consumers: FileContextConsumers,
    ) {
        if !consumers.any() {
            return;
        }
        let key = ProcessFileKey {
            trace_id,
            process: self.intern_process(&process),
        };
        if consumers.mcp_stdio {
            self.mcp_stdio.exit_process(&key, observed_ktime_ns);
        }
        if consumers.file_paths {
            self.processes.remove(&key);
        }
    }

    pub(crate) fn remove_trace(&mut self, trace_id: TraceId) {
        self.processes.retain(|key, _| key.trace_id != trace_id);
        self.pending.retain(|key, _| key.trace_id != trace_id);
        self.mcp_stdio.remove_trace(trace_id);
    }

    pub(crate) fn resolve_fd_path(
        &self,
        trace_id: TraceId,
        process: &ProcessObservation,
        fd: u32,
    ) -> Option<String> {
        if !self.file_capture_enabled || fd == FILE_FD_MISSING {
            return None;
        }
        let key = ProcessFileKey {
            trace_id,
            process: self.intern_process(process),
        };
        self.processes
            .get(&key)
            .and_then(|state| state.fds.get(&fd))
            .cloned()
    }

    pub(crate) fn resolve_file_descriptor(
        &self,
        trace_id: TraceId,
        process: &ProcessObservation,
        fd: u32,
    ) -> Option<ResolvedFileDescriptor> {
        if !self.file_capture_enabled || fd == FILE_FD_MISSING {
            return None;
        }
        let key = ProcessFileKey {
            trace_id,
            process: self.intern_process(process),
        };
        let state = self.processes.get(&key)?;
        Some(ResolvedFileDescriptor {
            path: state.fds.get(&fd)?.clone(),
            creation_requested: state.creation_requested_fds.contains(&fd),
        })
    }

    pub(super) fn record_ipc_fd_pair(
        &mut self,
        event: &KernelFilePathEvent,
        process: ProcessObservation,
        consumers: FileContextConsumers,
    ) -> bool {
        if event.kind != crate::decode::FILE_EVENT_CONTEXT
            || event.phase != FILE_PHASE_EXIT
            || event.result != 0
        {
            return false;
        }
        let Some(kind) = ipc_kind_from_event(event) else {
            return false;
        };
        if event.fd == FILE_FD_MISSING {
            return true;
        }
        let Some(peer_fd) = u32::try_from(event.arg0).ok() else {
            return true;
        };
        let process_key = ProcessFileKey {
            trace_id: event.trace_id,
            process: self.intern_process(&process),
        };
        if consumers.file_paths {
            let state = self.processes.entry(process_key.clone()).or_default();
            state.fds.remove(&event.fd);
            state.fds.remove(&peer_fd);
            state.creation_requested_fds.remove(&event.fd);
            state.creation_requested_fds.remove(&peer_fd);
        }
        if consumers.mcp_stdio {
            self.mcp_stdio.record_pair(
                &process_key,
                kind,
                event.fd,
                peer_fd,
                ipc_pair_close_on_exec(event),
                event.observed_ktime_ns,
            );
        }
        true
    }

    fn apply_successful_exit(
        &mut self,
        process_key: &ProcessFileKey,
        syscall: &KernelFilePathEvent,
        result: i64,
        primary_path: &PathResolution,
        secondary_path: &Option<PathResolution>,
    ) -> Option<String> {
        if syscall.aux == FILE_SYSCALL_CLOSE {
            let fd = syscall.arg0 as u32;
            let state = self.ensure_process(process_key);
            state.creation_requested_fds.remove(&fd);
            return state.fds.remove(&fd);
        }
        if result < 0 {
            return None;
        }
        match syscall.aux {
            FILE_SYSCALL_OPEN | FILE_SYSCALL_OPENAT | FILE_SYSCALL_CREAT | FILE_SYSCALL_OPENAT2 => {
                let fd = u32::try_from(result).ok()?;
                let track_fd = syscall.path_flags & PATH_FLAG_DIRECTORY != 0
                    || self.file_collection.fd_mutations;
                if !track_fd {
                    let state = self.ensure_process(process_key);
                    state.fds.remove(&fd);
                    state.creation_requested_fds.remove(&fd);
                    return None;
                }
                let creation_requested = open_requests_creation(syscall);
                let Some(path) = resolved_absolute_path(primary_path) else {
                    let state = self.ensure_process(process_key);
                    state.fds.remove(&fd);
                    state.creation_requested_fds.remove(&fd);
                    return None;
                };
                let state = self.ensure_process(process_key);
                state.fds.insert(fd, path.clone());
                if creation_requested {
                    state.creation_requested_fds.insert(fd);
                } else {
                    state.creation_requested_fds.remove(&fd);
                }
                Some(path)
            }
            FILE_SYSCALL_CLOSE_RANGE => {
                self.apply_close_range_exit(process_key, syscall);
                None
            }
            FILE_SYSCALL_DUP | FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 => {
                self.apply_dup_like_exit(process_key, syscall, result);
                None
            }
            FILE_SYSCALL_FCNTL if fcntl_duplicates_fd(syscall) => {
                self.apply_dup_like_exit(process_key, syscall, result);
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
                    syscall.arg0 as u32,
                ) {
                    self.ensure_process(process_key).cwd = Some(path);
                }
                None
            }
            FILE_SYSCALL_RENAME | FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => {
                self.apply_rename(
                    primary_path,
                    secondary_path,
                    syscall.aux == FILE_SYSCALL_RENAMEAT2 && syscall.arg4 & 2 != 0,
                );
                None
            }
            FILE_SYSCALL_MMAP | FILE_SYSCALL_FTRUNCATE => {
                self.resolve_fd_path(process_key.trace_id, &process_key.process, syscall.fd)
            }
            _ => None,
        }
    }

    fn apply_dup_like_exit(
        &mut self,
        process_key: &ProcessFileKey,
        syscall: &KernelFilePathEvent,
        result: i64,
    ) {
        let source = self.resolve_file_descriptor(
            process_key.trace_id,
            &process_key.process,
            syscall.arg0 as u32,
        );
        let Some(target_fd) = dup_target_fd(syscall, result) else {
            return;
        };
        let state = self.ensure_process(process_key);
        if let Some(source) = source {
            state.fds.insert(target_fd, source.path);
            if source.creation_requested {
                state.creation_requested_fds.insert(target_fd);
            } else {
                state.creation_requested_fds.remove(&target_fd);
            }
        } else {
            state.fds.remove(&target_fd);
            state.creation_requested_fds.remove(&target_fd);
        }
    }

    fn apply_close_range_exit(
        &mut self,
        process_key: &ProcessFileKey,
        syscall: &KernelFilePathEvent,
    ) {
        let first = syscall.arg0 as u32;
        let last = syscall.arg1 as u32;
        let close_on_exec = syscall.arg2 & CLOSE_RANGE_CLOEXEC != 0;
        if close_on_exec {
            return;
        }
        let state = self.ensure_process(process_key);
        state.fds.retain(|fd, _| *fd < first || *fd > last);
        state
            .creation_requested_fds
            .retain(|fd| *fd < first || *fd > last);
    }

    fn apply_rename(
        &mut self,
        primary_path: &PathResolution,
        secondary_path: &Option<PathResolution>,
        exchange: bool,
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
                } else if exchange && fd_path.as_str() == target.as_str() {
                    *fd_path = source.clone();
                }
            }
        }
    }

    fn resolve_primary_path(
        &mut self,
        process_key: &ProcessFileKey,
        event: &KernelFilePathEvent,
        result: i64,
    ) -> PathResolution {
        let raw = path_string(&event.path, event.path_flags);
        let dirfd = primary_dirfd(event);
        self.resolve_path(process_key, raw, dirfd, result)
    }

    fn resolve_secondary_path(
        &mut self,
        process_key: &ProcessFileKey,
        event: &KernelFilePathEvent,
        result: i64,
    ) -> Option<PathResolution> {
        let raw = path_string(&event.secondary_path, event.secondary_path_flags)?;
        let dirfd = secondary_dirfd(event);
        Some(self.resolve_path(process_key, Some(raw), dirfd, result))
    }

    fn resolve_path(
        &mut self,
        process_key: &ProcessFileKey,
        raw: Option<String>,
        dirfd: Option<u32>,
        result: i64,
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
                raw: Some(raw_path),
                resolved: Some(resolved),
                source: "absolute",
            };
        }
        // EBADF rejects resolution through an explicit directory FD. For
        // two-dirfd operations it does not identify which FD is invalid.
        if result == -i64::from(libc::EBADF) && dirfd.is_some_and(|fd| fd as i32 != libc::AT_FDCWD)
        {
            return PathResolution {
                raw: Some(raw_path),
                resolved: None,
                source: "unresolved_relative",
            };
        }
        let base = match dirfd {
            Some(fd) if fd as i32 != libc::AT_FDCWD => {
                self.resolve_fd_path(process_key.trace_id, &process_key.process, fd)
            }
            _ => self.ensure_process(process_key).cwd.clone(),
        };
        let Some(base) = base else {
            return PathResolution {
                raw: Some(raw_path),
                resolved: None,
                source: "unresolved_relative",
            };
        };
        // `base` is already normalized and `raw_path` is relative here, so a
        // plain join string is equivalent to PathBuf::join and avoids two
        // intermediate allocations before normalization.
        let resolved = lexically_normalize_path(&format!("{base}/{raw_path}"));
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

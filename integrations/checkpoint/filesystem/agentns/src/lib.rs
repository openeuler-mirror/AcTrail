//! AgentNS checkpoint backend on RNS, the current AgentNS: the kernel journals every change the
//! Trace's processes make, and a save seals the current turn into an event to restore to. RNS
//! covers the whole filesystem those processes write to, so the backend takes one scope rooted at
//! `/`, minus that scope's excludes.
//! It requires tree-only capture and preserving the processes the coordinator stopped.

mod config;
mod library;

use std::collections::BTreeSet;
use std::ffi::{CString, c_int};
use std::fmt::Display;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use abi_stable::export_root_module;
use abi_stable::prefix_type::PrefixTypeTrait;
use abi_stable::sabi_extern_fn;
use abi_stable::sabi_trait::TD_Opaque;
use abi_stable::std_types::{RResult, RString, RVec};
use checkpoint::{
    BackendConfig, BackendError, BackendErrorKind, BackendKind, CheckpointBackend,
    CheckpointModule, CheckpointModuleRef, RestorableCheckpointBackend,
    RestorableCheckpointBackend_TO, RestoreFailure, SnapshotContext, SnapshotId, TraceProcess,
};

use crate::config::AgentNsConfig;
use crate::library::{RestoreState, RestoreStep, RnsLibrary, Session};

#[export_root_module]
fn module() -> CheckpointModuleRef {
    CheckpointModule {
        create,
        kind: BackendKind::Filesystem,
    }
    .leak_into_prefix()
}

#[sabi_extern_fn]
fn create(config: BackendConfig) -> RResult<CheckpointBackend, BackendError> {
    AgentNsBackend::new(&config.json)
        .map(|backend| RestorableCheckpointBackend_TO::from_value(backend, TD_Opaque))
        .into()
}

pub struct AgentNsBackend {
    config: AgentNsConfig,
    library: RnsLibrary,
    connection: Option<Connection>,
}

/// The RNS area of this instance, set by `connect`.
struct Connection {
    /// Private directory whose path names the RNS area.
    workspace: PathBuf,
    session: String,
    /// Events that are neither deleted nor invalidated.
    events: BTreeSet<u64>,
    /// A retained request, or an attempt whose reply was lost. Rejected STARTs leave none.
    unfinished_restore: Option<PendingRestore>,
    /// RNS discarded the area, so a retried disconnect has only the directory left to remove.
    area_discarded: bool,
}

#[derive(Clone, Copy)]
struct PendingRestore {
    request: [u64; 2],
    event: u64,
    /// Checkpoints created after an uncertain reply are outside this request's original range.
    last_event: u64,
    /// Only a reliable RNS phase permits CONTINUE; otherwise query the original request first.
    accepted: bool,
}

impl Connection {
    fn session(&self) -> Result<Session, BackendError> {
        Session::new(&self.workspace, &self.session)
    }

    fn record_restore(&mut self, pending: PendingRestore, state: RestoreState) -> RVec<SnapshotId> {
        match state {
            RestoreState::Rejected => return RVec::new(),
            RestoreState::Unknown => {
                self.unfinished_restore = Some(pending);
                return RVec::new();
            }
            RestoreState::Unfinished => {
                self.unfinished_restore = Some(PendingRestore {
                    accepted: true,
                    ..pending
                });
            }
            RestoreState::Done => self.unfinished_restore = None,
        }
        // Only an accepted rollback commits these later events to being consumed. An unknown
        // reply leaves them intact until the original request's phase is confirmed.
        let later = self.events.split_off(&(pending.event + 1));
        let mut invalidated = RVec::new();
        for event in later {
            if event <= pending.last_event {
                invalidated.push(snapshot_id(event));
            } else {
                self.events.insert(event);
            }
        }
        invalidated
    }
}

impl AgentNsBackend {
    pub fn new(config: &str) -> Result<Self, BackendError> {
        let config = AgentNsConfig::parse(config)?;
        let library = RnsLibrary::load(&config.library_path)?;
        fs::create_dir_all(&config.area_root)
            .and_then(|()| {
                fs::set_permissions(&config.area_root, fs::Permissions::from_mode(0o700))
            })
            .map_err(|error| io_error(config.area_root.display(), error))?;
        Ok(Self {
            config,
            library,
            connection: None,
        })
    }

    fn exclusions(&self, excludes: &[RString]) -> Result<Vec<CString>, BackendError> {
        let mut paths: Vec<PathBuf> = self.config.extra_excluded_paths.clone();
        paths.push(self.config.area_root.clone());
        paths.extend(excludes.iter().map(|path| PathBuf::from(path.as_str())));
        paths.sort();
        paths.dedup();
        paths
            .iter()
            .map(|path| {
                CString::new(path.as_os_str().as_bytes())
                    .map_err(|_| invalid_scope(format!("{} contains NUL", path.display())))
            })
            .collect()
    }

    fn bind(&mut self, context: &SnapshotContext) -> Result<(), BackendError> {
        let [scope] = context.scopes.as_slice() else {
            return Err(invalid_scope(
                "AgentNS protects the whole filesystem in one scope rooted at /",
            ));
        };
        if scope.root != "/" {
            return Err(invalid_scope(format!(
                "scope {} is rooted at {}; AgentNS covers the whole filesystem, so the root must be /",
                scope.id, scope.root
            )));
        }
        let pid = require_process(&context.trace_process)?;
        let exclusions = self.exclusions(&scope.excludes)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let workspace = self
            .config
            .area_root
            .join(format!("{}-{nanos}", context.trace_id));
        fs::create_dir(&workspace)
            .and_then(|()| fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700)))
            .map_err(|error| io_error(workspace.display(), error))?;
        let connection = Connection {
            workspace,
            session: format!("actrail-{}", context.trace_id),
            events: BTreeSet::new(),
            unfinished_restore: None,
            area_discarded: false,
        };
        let session = connection.session()?;
        let started = self
            .library
            .create(&session, pid, &exclusions)
            .map_err(|error| agentns_error("create the RNS session", error))
            .and_then(|()| {
                self.library.attach(&session, pid).map_err(|error| {
                    let _ = self.library.discard(&session);
                    agentns_error("attach the Trace root", error)
                })
            });
        if let Err(error) = started {
            // The launch fails, so a failure here only leaves an empty directory behind.
            let _ = fs::remove_dir_all(&connection.workspace);
            return Err(error);
        }
        self.connection = Some(connection);
        Ok(())
    }

    fn save(&mut self) -> Result<SnapshotId, BackendError> {
        let connection = connected(&mut self.connection)?;
        let event = self
            .library
            .checkpoint(&connection.session()?, "actrail save")
            .map_err(|error| agentns_error("seal the checkpoint", error))?;
        connection.events.insert(event);
        Ok(snapshot_id(event))
    }

    /// Each restore is an RNS request that stays recorded until it is done, so a request that
    /// stops short of done is continued by the next restore.
    fn roll_back(&mut self, id: &SnapshotId) -> Result<RVec<SnapshotId>, RestoreFailure> {
        let connection = connected(&mut self.connection)?;
        let event = parse_id(id)?;
        let session = connection.session()?;
        let mut invalidated = finish_unfinished_restore(&self.library, connection, &session)?;
        let result = (|| {
            // Resolving an earlier uncertain request can invalidate the newly selected target.
            if !connection.events.contains(&event) {
                return Err(unknown_snapshot(id));
            }
            // Seal the open tail so the rollback includes changes since the last save.
            self.library
                .checkpoint(&session, "actrail restore")
                .map_err(|error| agentns_error("seal the changes since the last save", error))?;
            let pending = PendingRestore {
                request: new_request()?,
                event,
                last_event: *connection.events.last().expect("the target is a saved event"),
                accepted: false,
            };
            let call =
                self.library
                    .restore(&session, pending.request, RestoreStep::Start { event });
            invalidated.extend(connection.record_restore(pending, call.state));
            call.result
                .map_err(|reason| failed(format!("restore the checkpoint: {reason}")))
        })();
        match result {
            Ok(()) => Ok(invalidated),
            Err(error) => Err(RestoreFailure {
                error,
                invalidated,
            }),
        }
    }

    /// The journal keeps every change from the first event on, so deleting one checkpoint frees
    /// nothing by itself; `disconnect` frees the area.
    fn remove(&mut self, id: &SnapshotId) -> Result<(), BackendError> {
        connected(&mut self.connection)?
            .events
            .remove(&parse_id(id)?);
        Ok(())
    }

    /// Frees the area with every history in it. A restore that stopped short of done is dropped
    /// with it: the files are being given up, and RNS discards an idle request along with the
    /// area.
    fn release(&mut self) -> Result<(), BackendError> {
        let connection = connected(&mut self.connection)?;
        if !connection.area_discarded {
            self.library
                .discard(&connection.session()?)
                .map_err(|error| agentns_error("free the RNS area", error))?;
            connection.area_discarded = true;
        }
        // The connection owns the area directory, which RNS leaves in place.
        fs::remove_dir_all(&connection.workspace)
            .map_err(|error| io_error(connection.workspace.display(), error))?;
        self.connection = None;
        Ok(())
    }
}

impl RestorableCheckpointBackend for AgentNsBackend {
    fn connect(&mut self, context: &SnapshotContext) -> RResult<(), BackendError> {
        self.bind(context).into()
    }

    fn snapshot(&mut self) -> RResult<SnapshotId, BackendError> {
        self.save().into()
    }

    fn restore(&mut self, snapshot_id: &SnapshotId) -> RResult<RVec<SnapshotId>, RestoreFailure> {
        self.roll_back(snapshot_id).into()
    }

    fn delete(&mut self, snapshot_id: &SnapshotId) -> RResult<(), BackendError> {
        self.remove(snapshot_id).into()
    }

    fn disconnect(&mut self) -> RResult<(), BackendError> {
        self.release().into()
    }
}

fn connected(connection: &mut Option<Connection>) -> Result<&mut Connection, BackendError> {
    connection
        .as_mut()
        .ok_or_else(|| backend_error("not_connected", "the backend is not connected"))
}

/// Carries a restore request that stopped short of done on to done.
fn finish_unfinished_restore(
    library: &RnsLibrary,
    connection: &mut Connection,
    session: &Session,
) -> Result<RVec<SnapshotId>, RestoreFailure> {
    let Some(mut pending) = connection.unfinished_restore else {
        return Ok(RVec::new());
    };
    let mut invalidated = RVec::new();
    let call = if pending.accepted {
        library.restore(session, pending.request, RestoreStep::Continue)
    } else {
        let query = library.restore(session, pending.request, RestoreStep::Query);
        match query.state {
            RestoreState::Unfinished => {
                pending.accepted = true;
                invalidated.extend(connection.record_restore(pending, query.state));
                library.restore(session, pending.request, RestoreStep::Continue)
            }
            RestoreState::Rejected if query.errno == Some(libc::ENOENT) => {
                connection.unfinished_restore = None;
                return Ok(invalidated);
            }
            _ => query,
        }
    };
    invalidated.extend(connection.record_restore(pending, call.state));
    match call.result {
        Ok(()) => Ok(invalidated),
        Err(reason) => Err(RestoreFailure {
            error: failed(format!("finish the unfinished restore: {reason}")),
            invalidated,
        }),
    }
}

fn snapshot_id(event: u64) -> SnapshotId {
    SnapshotId(event.to_string().into_bytes().into())
}

fn parse_id(id: &SnapshotId) -> Result<u64, BackendError> {
    std::str::from_utf8(&id.0)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| unknown_snapshot(id))
}

fn unknown_snapshot(id: &SnapshotId) -> BackendError {
    BackendError::new(
        BackendErrorKind::InvalidSnapshot,
        format!("unknown snapshot {}", String::from_utf8_lossy(&id.0)),
    )
}

/// A fresh nonzero restore request id; RNS needs it unique within the area.
fn new_request() -> Result<[u64; 2], BackendError> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|error| io_error("/dev/urandom", error))?;
    let [low, high] = [&bytes[..8], &bytes[8..]]
        .map(|half| u64::from_ne_bytes(half.try_into().expect("eight bytes")));
    Ok([low | 1, high])
}

/// The Trace root, checked by start time so a reused PID is not attributed.
fn require_process(process: &TraceProcess) -> Result<c_int, BackendError> {
    let own_namespace =
        fs::read_link("/proc/self/ns/mnt").map_err(|error| io_error("/proc/self/ns/mnt", error))?;
    if process.mount_namespace.as_str() != own_namespace.to_string_lossy() {
        return Err(backend_error(
            "unsupported_namespace",
            "the Agent must share the backend's mount namespace",
        ));
    }
    let path = format!("/proc/{}/stat", process.host_pid);
    let stat = fs::read_to_string(&path).map_err(|error| io_error(&path, error))?;
    let start_time = stat
        .rfind(')')
        .and_then(|end| stat[end + 1..].split_whitespace().nth(19))
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| backend_error("io", format!("{path} has no start time")))?;
    if start_time != process.start_time_ticks {
        return Err(backend_error(
            "process_gone",
            format!("process {} is no longer the Trace root", process.host_pid),
        ));
    }
    c_int::try_from(process.host_pid).map_err(|_| {
        backend_error(
            "process_gone",
            format!("pid {} does not fit", process.host_pid),
        )
    })
}

fn invalid_scope(message: impl Into<String>) -> BackendError {
    backend_error("invalid_scope", message)
}

fn failed(message: String) -> BackendError {
    backend_error("agentns", message)
}

fn agentns_error(action: &str, error: std::io::Error) -> BackendError {
    failed(format!("{action}: {error}"))
}

pub(crate) fn backend_error(code: &str, message: impl Into<String>) -> BackendError {
    let kind = if code.starts_with("invalid_") {
        BackendErrorKind::InvalidConfiguration
    } else {
        BackendErrorKind::OperationFailed
    };
    BackendError::new(kind, format!("{code}: {}", message.into()))
}

pub(crate) fn io_error(context: impl Display, error: std::io::Error) -> BackendError {
    backend_error("io", format!("{context}: {error}"))
}

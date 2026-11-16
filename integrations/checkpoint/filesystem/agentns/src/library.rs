//! The part of the RNS C SDK (`rns_api.h`, the current AgentNS) the backend uses, loaded from
//! `librns.so` when an instance is created. The structures mirror the header.

use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use checkpoint::BackendError;
use libloading::Library;

use crate::backend_error;

const TASK_THREAD_GROUP: c_uint = 1;
const CREATE_AGENT_TREE_ONLY: c_uint = 2;
const RESTORE_PRESERVE_STOPPED: c_uint = 2;
const REQUIRED_CAPABILITIES: u64 = (1 << 0) | (1 << 1);
const RESTORE_START: c_uint = 1;
const RESTORE_QUERY: c_uint = 2;
const RESTORE_CONTINUE: c_uint = 3;
const RESTORE_EVENT: c_uint = 2;
const RESTORE_DONE: u32 = 4;
const VALID_PHASE: u64 = 1 << 1;
const VALID_ERROR: u64 = 1 << 5;

#[repr(C)]
struct CreateOptions {
    main_pid: c_int,
    exclude_paths: *const *const c_char,
    exclude_count: usize,
    workspace_fd: c_int,
    use_workspace_fd: bool,
    area_name: *const c_char,
}

#[repr(C)]
struct RestoreTarget {
    action: c_uint,
    request_id: [u64; 2],
    kind: c_uint,
    turns_back: u32,
    target_event_id: u64,
}

#[repr(C)]
struct RestoreResult {
    valid_fields: u64,
    phase: u32,
    operation_errno: c_int,
    event_id_out: u64,
    unrestored_count: u64,
    overlap_count: u64,
    tasks: *mut c_void,
    task_capacity: usize,
    task_total_count: u64,
    task_written_count: usize,
    task_truncated: bool,
}

type CapabilitiesFn = unsafe extern "C" fn(*mut u64) -> c_int;
type CreateFn =
    unsafe extern "C" fn(*const c_char, *const c_char, *const CreateOptions, c_uint) -> c_int;
type AttachFn = unsafe extern "C" fn(*const c_char, *const c_char, c_int, c_uint, u64) -> c_int;
type CheckpointFn =
    unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *mut u64) -> c_int;
type RestoreFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const RestoreTarget,
    c_uint,
    *mut RestoreResult,
) -> c_int;
type DelFn = unsafe extern "C" fn(*const c_char) -> c_int;

/// One RNS session: the area its workspace names and the chain its session id names.
pub(crate) struct Session {
    workspace: CString,
    session: CString,
}

impl Session {
    pub(crate) fn new(workspace: &Path, session: &str) -> Result<Self, BackendError> {
        let nul = |what: &str| backend_error("invalid_session", format!("{what} contains NUL"));
        Ok(Self {
            workspace: CString::new(workspace.as_os_str().as_bytes())
                .map_err(|_| nul("the area directory"))?,
            session: CString::new(session).map_err(|_| nul("the session id"))?,
        })
    }
}

/// What a restore call does with its request.
pub(crate) enum RestoreStep {
    /// Starts the request: roll the area back to just after `event`.
    Start { event: u64 },
    /// Reads the original request after an unreliable reply, without changing any files.
    Query,
    /// Carries on with a request that stopped before it was done.
    Continue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RestoreState {
    Rejected,
    Unfinished,
    Done,
    /// The SDK could not publish a reliable result; query the original request before proceeding.
    Unknown,
}

pub(crate) struct RestoreCall {
    pub(crate) state: RestoreState,
    pub(crate) errno: Option<c_int>,
    pub(crate) result: Result<(), String>,
}

pub(crate) struct RnsLibrary {
    _library: Library,
    create: CreateFn,
    attach: AttachFn,
    checkpoint: CheckpointFn,
    restore: RestoreFn,
    del: DelFn,
}

impl RnsLibrary {
    pub(crate) fn load(path: &Path) -> Result<Self, BackendError> {
        let error = |error: libloading::Error| {
            backend_error(
                "library_unavailable",
                format!("{}: {error}", path.display()),
            )
        };
        // SAFETY: loading runs librns initializers, and the symbols are its public API with these
        // signatures (rns_api.h).
        unsafe {
            let library = Library::new(path).map_err(error)?;
            let capabilities = *library
                .get::<CapabilitiesFn>(b"rns_capabilities\0")
                .map_err(error)?;
            let mut supported = 0;
            check(capabilities(&mut supported)).map_err(|error| {
                backend_error(
                    "unsupported_runtime",
                    format!("query RNS capabilities: {error}"),
                )
            })?;
            if supported & REQUIRED_CAPABILITIES != REQUIRED_CAPABILITIES {
                return Err(backend_error(
                    "unsupported_runtime",
                    "RNS must support agent-tree-only capture and preserving stopped processes",
                ));
            }
            Ok(Self {
                create: *library
                    .get::<CreateFn>(b"rns_create_with_flags\0")
                    .map_err(error)?,
                attach: *library.get::<AttachFn>(b"rns_attach\0").map_err(error)?,
                checkpoint: *library.get::<CheckpointFn>(b"rns_ckpt\0").map_err(error)?,
                restore: *library.get::<RestoreFn>(b"rns_restore\0").map_err(error)?,
                del: *library.get::<DelFn>(b"rns_del\0").map_err(error)?,
                _library: library,
            })
        }
    }

    /// Creates the area and chain with `pid` as the session main. Its process tree follows the
    /// current event, captures outside `exclusions`, and does not attach IPC peers.
    pub(crate) fn create(
        &self,
        session: &Session,
        pid: c_int,
        exclusions: &[CString],
    ) -> io::Result<()> {
        let pointers: Vec<*const c_char> = exclusions.iter().map(|path| path.as_ptr()).collect();
        let options = CreateOptions {
            main_pid: pid,
            exclude_paths: pointers.as_ptr(),
            exclude_count: pointers.len(),
            workspace_fd: -1,
            use_workspace_fd: false,
            area_name: std::ptr::null(),
        };
        // SAFETY: the strings and `pointers` outlive the call, which only borrows them.
        check(unsafe {
            (self.create)(
                session.workspace.as_ptr(),
                session.session.as_ptr(),
                &options,
                CREATE_AGENT_TREE_ONLY,
            )
        })
    }

    /// Attributes `pid` and its existing threads to the session's current turn; tasks it forks
    /// later are attributed by RNS.
    pub(crate) fn attach(&self, session: &Session, pid: c_int) -> io::Result<()> {
        // SAFETY: the strings outlive the call.
        check(unsafe {
            (self.attach)(
                session.workspace.as_ptr(),
                session.session.as_ptr(),
                pid,
                TASK_THREAD_GROUP,
                0,
            )
        })
    }

    /// Seals the current turn and returns its event id.
    pub(crate) fn checkpoint(&self, session: &Session, label: &str) -> io::Result<u64> {
        let label = CString::new(label).map_err(io::Error::other)?;
        let mut event = 0;
        // SAFETY: the strings outlive the call and `event` is a valid out pointer.
        check(unsafe {
            (self.checkpoint)(
                session.workspace.as_ptr(),
                session.session.as_ptr(),
                label.as_ptr(),
                &mut event,
            )
        })?;
        Ok(event)
    }

    /// Preserves whether RNS retained this request, even when the call failed. A rejection before
    /// registration has no phase; a retained request reports its phase independently of errno.
    pub(crate) fn restore(
        &self,
        session: &Session,
        request: [u64; 2],
        step: RestoreStep,
    ) -> RestoreCall {
        let (action, kind, target_event_id) = match step {
            RestoreStep::Start { event } => (RESTORE_START, RESTORE_EVENT, event),
            RestoreStep::Query => (RESTORE_QUERY, 0, 0),
            RestoreStep::Continue => (RESTORE_CONTINUE, 0, 0),
        };
        let target = RestoreTarget {
            action,
            request_id: request,
            kind,
            turns_back: 0,
            target_event_id,
        };
        let mut result = RestoreResult {
            valid_fields: 0,
            phase: 0,
            operation_errno: 0,
            event_id_out: 0,
            unrestored_count: 0,
            overlap_count: 0,
            tasks: std::ptr::null_mut(),
            task_capacity: 0,
            task_total_count: 0,
            task_written_count: 0,
            task_truncated: false,
        };
        // SAFETY: the strings and `target` outlive the call, and `result` is a valid out pointer
        // that requests no task report.
        let returned = unsafe {
            (self.restore)(
                session.workspace.as_ptr(),
                session.session.as_ptr(),
                &target,
                RESTORE_PRESERVE_STOPPED,
                &mut result,
            )
        };
        let call = check(returned);
        let errno = call.as_ref().err().and_then(io::Error::raw_os_error);
        let phase = (result.valid_fields & VALID_PHASE != 0).then_some(result.phase);
        let state = match phase {
            Some(RESTORE_DONE) => RestoreState::Done,
            Some(_) => RestoreState::Unfinished,
            None if call.is_err() && !matches!(errno, Some(libc::EFAULT | libc::EPROTO)) => {
                RestoreState::Rejected
            }
            // EFAULT loses the reply after possible effects; EPROTO reports unreliable output.
            None => RestoreState::Unknown,
        };
        if call.is_ok() && phase == Some(RESTORE_DONE) {
            return RestoreCall {
                state,
                errno,
                result: Ok(()),
            };
        }
        let mut stopped = match phase {
            Some(phase) => format!("the restore stopped in phase {phase}"),
            None => "the restore reported no phase".to_string(),
        };
        if let Err(error) = call {
            stopped.push_str(&format!(": {error}"));
        }
        if result.valid_fields & VALID_ERROR != 0 && result.operation_errno != 0 {
            let error = io::Error::from_raw_os_error(result.operation_errno);
            stopped.push_str(&format!("; the operation reports {error}"));
        }
        RestoreCall {
            state,
            errno,
            result: Err(stopped),
        }
    }

    /// Discards the area and every history in it, also while processes of the session still run.
    pub(crate) fn discard(&self, session: &Session) -> io::Result<()> {
        // SAFETY: the string outlives the call.
        check(unsafe { (self.del)(session.workspace.as_ptr()) })
    }
}

fn check(result: c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

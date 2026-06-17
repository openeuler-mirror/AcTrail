//! Decoding raw file syscall records into collector events.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::CollectorName;

use crate::decode::{
    DecodeError, FILE_EVENT_CONTEXT, FILE_EVENT_MMAP, FILE_EVENT_OPEN, resolve_event_identity,
};
use crate::loader::KernelFilePathEvent;
use crate::maps::BindingStateMap;
use crate::procfs::ProcfsIdentityReader;

use super::state::{
    FILE_FD_MISSING, FILE_PHASE_EXIT, FILE_SYSCALL_CHDIR, FILE_SYSCALL_CLOSE, FILE_SYSCALL_CREAT,
    FILE_SYSCALL_DUP, FILE_SYSCALL_DUP2, FILE_SYSCALL_DUP3, FILE_SYSCALL_FCHDIR,
    FILE_SYSCALL_FCNTL, FILE_SYSCALL_FTRUNCATE, FILE_SYSCALL_MKDIR, FILE_SYSCALL_MKDIRAT,
    FILE_SYSCALL_MMAP, FILE_SYSCALL_OPEN, FILE_SYSCALL_OPENAT, FILE_SYSCALL_OPENAT2,
    FILE_SYSCALL_RENAME, FILE_SYSCALL_RENAMEAT, FILE_SYSCALL_RENAMEAT2, FILE_SYSCALL_RMDIR,
    FILE_SYSCALL_TRUNCATE, FILE_SYSCALL_UNLINK, FILE_SYSCALL_UNLINKAT, FileSyscallOutcome,
    FileTracker, PATH_FLAG_FAULT, PATH_FLAG_TRUNCATED,
};

pub(in crate::decode) fn decode(
    event: KernelFilePathEvent,
    bindings: &BindingStateMap,
    identity_reader: &ProcfsIdentityReader,
    tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if event.kind == FILE_EVENT_MMAP {
        if !bindings.trace_has_capability(event.trace_id, &Capability::FsMmap) {
            return Ok(None);
        }
    } else if !bindings.trace_has_capability(event.trace_id, &Capability::FsAccessBasic) {
        return Ok(None);
    }

    let is_exit = event.phase == FILE_PHASE_EXIT;
    let outcome = tracker.record(event);
    let Some(outcome) = outcome else {
        return Ok(None);
    };
    if outcome.enter.kind == FILE_EVENT_CONTEXT && outcome.enter.aux != FILE_SYSCALL_CLOSE {
        return Ok(None);
    }
    if outcome.enter.kind == FILE_EVENT_MMAP && !mmap_is_shared_writable(&outcome) {
        return Ok(None);
    }
    if !is_exit {
        return Ok(None);
    }

    let identity = resolve_event_identity(
        outcome.enter.pid,
        outcome.enter.pid_generation,
        bindings,
        identity_reader,
    )
    .map_err(|error| DecodeError::new("file_identity", error))?;
    let operation = file_operation(&outcome);
    let path = outcome
        .primary_path
        .resolved
        .clone()
        .or_else(|| outcome.primary_path.raw.clone())
        .or_else(|| outcome.fd_path.clone());
    let metadata = file_metadata(&outcome, operation);

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process: identity,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::File {
            operation: operation.to_string(),
            path,
            metadata,
        },
    }))
}

fn file_operation(outcome: &FileSyscallOutcome) -> &'static str {
    match outcome.enter.kind {
        FILE_EVENT_OPEN if open_truncates(&outcome.enter) => "truncate",
        FILE_EVENT_OPEN => "open",
        crate::decode::FILE_EVENT_CONTEXT if outcome.enter.aux == FILE_SYSCALL_CLOSE => "close",
        crate::decode::FILE_EVENT_CONTEXT => "context",
        crate::decode::FILE_EVENT_UNLINK if unlinkat_removes_directory(&outcome.enter) => "rmdir",
        crate::decode::FILE_EVENT_UNLINK => "unlink",
        crate::decode::FILE_EVENT_RENAME => "rename",
        crate::decode::FILE_EVENT_MKDIR => "mkdir",
        crate::decode::FILE_EVENT_RMDIR => "rmdir",
        crate::decode::FILE_EVENT_TRUNCATE => "truncate",
        FILE_EVENT_MMAP => "mmap_shared",
        _ => "unknown",
    }
}

fn file_metadata(outcome: &FileSyscallOutcome, operation: &str) -> BTreeMap<String, String> {
    let event = &outcome.enter;
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("result".to_string(), normalized_result(outcome).to_string()),
        ("syscall".to_string(), syscall_name(event.aux).to_string()),
        (
            "path_max_bytes".to_string(),
            event.path_max_bytes.to_string(),
        ),
        (
            "path_resolution".to_string(),
            outcome.primary_path.source.to_string(),
        ),
    ]);
    if let Some(raw_path) = &outcome.primary_path.raw {
        metadata.insert("raw_path".to_string(), raw_path.clone());
    }
    insert_userspace_retry_metadata(&mut metadata, "path", &outcome.primary_path);
    if let Some(fd_path) = &outcome.fd_path {
        metadata.insert("fd_target".to_string(), fd_path.clone());
    }
    insert_fd_metadata(&mut metadata, outcome);
    insert_path_metadata(&mut metadata, "path", event.path_size, event.path_flags);
    if let Some(target_path) = &outcome.secondary_path {
        if let Some(path) = target_path.resolved.as_ref().or(target_path.raw.as_ref()) {
            metadata.insert("target_path".to_string(), path.clone());
        }
        if let Some(raw_path) = &target_path.raw {
            metadata.insert("raw_target_path".to_string(), raw_path.clone());
        }
        insert_userspace_retry_metadata(&mut metadata, "target_path", target_path);
        metadata.insert(
            "target_path_resolution".to_string(),
            target_path.source.to_string(),
        );
        insert_path_metadata(
            &mut metadata,
            "target_path",
            event.secondary_path_size,
            event.secondary_path_flags,
        );
    }
    insert_syscall_args(&mut metadata, outcome);
    metadata
}

fn insert_userspace_retry_metadata(
    metadata: &mut BTreeMap<String, String>,
    prefix: &str,
    path: &super::state::PathResolution,
) {
    if !path.userspace_retry {
        return;
    }
    metadata.insert(
        format!("{prefix}_retry_source"),
        "process_vm_readv".to_string(),
    );
    metadata.insert(
        format!("{prefix}_retry_truncated"),
        path.userspace_retry_truncated.to_string(),
    );
}

fn insert_fd_metadata(metadata: &mut BTreeMap<String, String>, outcome: &FileSyscallOutcome) {
    let event = &outcome.enter;
    let fd = match event.aux {
        FILE_SYSCALL_OPEN | FILE_SYSCALL_OPENAT | FILE_SYSCALL_CREAT | FILE_SYSCALL_OPENAT2
            if outcome.result >= 0 =>
        {
            u32::try_from(outcome.result).ok()
        }
        FILE_SYSCALL_MMAP | FILE_SYSCALL_FTRUNCATE if event.fd != FILE_FD_MISSING => Some(event.fd),
        FILE_SYSCALL_CLOSE => Some(event.arg0 as u32),
        _ => None,
    };
    if let Some(fd) = fd {
        metadata.insert("fd".to_string(), fd.to_string());
    }
}

fn insert_path_metadata(
    metadata: &mut BTreeMap<String, String>,
    prefix: &str,
    size: u32,
    flags: u32,
) {
    metadata.insert(format!("{prefix}_captured_size"), size.to_string());
    metadata.insert(
        format!("{prefix}_truncated"),
        flag_enabled(flags, PATH_FLAG_TRUNCATED).to_string(),
    );
    if flag_enabled(flags, PATH_FLAG_FAULT) {
        metadata.insert(format!("{prefix}_read_fault"), "true".to_string());
    }
}

fn insert_syscall_args(metadata: &mut BTreeMap<String, String>, outcome: &FileSyscallOutcome) {
    let event = &outcome.enter;
    match event.aux {
        FILE_SYSCALL_OPEN => {
            metadata.insert("flags".to_string(), event.arg1.to_string());
            metadata.insert("mode".to_string(), event.arg2.to_string());
        }
        FILE_SYSCALL_OPENAT => {
            metadata.insert("dirfd".to_string(), (event.arg0 as i32).to_string());
            metadata.insert("flags".to_string(), event.arg2.to_string());
            if open_truncates(event) {
                metadata.insert("truncate_source".to_string(), "openat_o_trunc".to_string());
            }
        }
        FILE_SYSCALL_OPENAT2 => {
            metadata.insert("dirfd".to_string(), (event.arg0 as i32).to_string());
            metadata.insert("flags".to_string(), event.arg2.to_string());
            metadata.insert("mode".to_string(), event.arg3.to_string());
            metadata.insert("resolve".to_string(), event.arg4.to_string());
            metadata.insert("how_size".to_string(), event.arg5.to_string());
            if open_truncates(event) {
                metadata.insert("truncate_source".to_string(), "openat2_o_trunc".to_string());
            }
        }
        FILE_SYSCALL_CREAT => {
            metadata.insert("mode".to_string(), event.arg1.to_string());
        }
        FILE_SYSCALL_UNLINKAT => {
            metadata.insert("dirfd".to_string(), (event.arg0 as i32).to_string());
            metadata.insert("flags".to_string(), event.arg2.to_string());
        }
        FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => {
            metadata.insert("old_dirfd".to_string(), (event.arg0 as i32).to_string());
            metadata.insert("new_dirfd".to_string(), (event.arg2 as i32).to_string());
        }
        FILE_SYSCALL_MKDIR => {
            metadata.insert("mode".to_string(), event.arg1.to_string());
        }
        FILE_SYSCALL_MKDIRAT => {
            metadata.insert("dirfd".to_string(), (event.arg0 as i32).to_string());
            metadata.insert("mode".to_string(), event.arg2.to_string());
        }
        FILE_SYSCALL_TRUNCATE | FILE_SYSCALL_FTRUNCATE => {
            metadata.insert("length".to_string(), event.arg1.to_string());
        }
        FILE_SYSCALL_MMAP => {
            metadata.insert("address_hint".to_string(), event.arg0.to_string());
            metadata.insert("length".to_string(), event.arg1.to_string());
            metadata.insert("protection".to_string(), event.arg2.to_string());
            metadata.insert("flags".to_string(), event.arg3.to_string());
            if outcome.result >= 0 {
                metadata.insert("mapped_address".to_string(), outcome.result.to_string());
            }
            metadata.insert("offset".to_string(), event.arg5.to_string());
            metadata.insert(
                "shared".to_string(),
                mmap_is_shared_writable(outcome).to_string(),
            );
        }
        FILE_SYSCALL_CLOSE | FILE_SYSCALL_DUP | FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3
        | FILE_SYSCALL_FCNTL | FILE_SYSCALL_CHDIR | FILE_SYSCALL_FCHDIR | FILE_SYSCALL_RENAME
        | FILE_SYSCALL_UNLINK | FILE_SYSCALL_RMDIR => {}
        _ => {}
    }
}

fn normalized_result(outcome: &FileSyscallOutcome) -> i64 {
    if outcome.enter.aux == FILE_SYSCALL_MMAP && outcome.result >= 0 {
        return 0;
    }
    outcome.result
}

fn flag_enabled(flags: u32, flag: u32) -> bool {
    flags & flag != 0
}

fn syscall_name(raw: u32) -> &'static str {
    match raw {
        FILE_SYSCALL_OPEN => "open",
        FILE_SYSCALL_OPENAT => "openat",
        FILE_SYSCALL_OPENAT2 => "openat2",
        FILE_SYSCALL_CREAT => "creat",
        FILE_SYSCALL_UNLINK => "unlink",
        FILE_SYSCALL_UNLINKAT => "unlinkat",
        FILE_SYSCALL_RENAME => "rename",
        FILE_SYSCALL_RENAMEAT => "renameat",
        FILE_SYSCALL_RENAMEAT2 => "renameat2",
        FILE_SYSCALL_MKDIR => "mkdir",
        FILE_SYSCALL_MKDIRAT => "mkdirat",
        FILE_SYSCALL_RMDIR => "rmdir",
        FILE_SYSCALL_TRUNCATE => "truncate",
        FILE_SYSCALL_FTRUNCATE => "ftruncate",
        FILE_SYSCALL_MMAP => "mmap",
        FILE_SYSCALL_CLOSE => "close",
        FILE_SYSCALL_DUP => "dup",
        FILE_SYSCALL_DUP2 => "dup2",
        FILE_SYSCALL_DUP3 => "dup3",
        FILE_SYSCALL_FCNTL => "fcntl",
        FILE_SYSCALL_CHDIR => "chdir",
        FILE_SYSCALL_FCHDIR => "fchdir",
        _ => "unknown",
    }
}

fn unlinkat_removes_directory(event: &KernelFilePathEvent) -> bool {
    event.aux == FILE_SYSCALL_UNLINKAT && event.arg2 & libc::AT_REMOVEDIR as u64 != 0
}

fn open_truncates(event: &KernelFilePathEvent) -> bool {
    match event.aux {
        FILE_SYSCALL_OPEN => event.arg1 & libc::O_TRUNC as u64 != 0,
        FILE_SYSCALL_OPENAT => event.arg2 & libc::O_TRUNC as u64 != 0,
        FILE_SYSCALL_OPENAT2 => event.arg2 & libc::O_TRUNC as u64 != 0,
        _ => false,
    }
}

fn mmap_is_shared_writable(outcome: &FileSyscallOutcome) -> bool {
    outcome.enter.aux == FILE_SYSCALL_MMAP
        && outcome.result >= 0
        && outcome.enter.arg2 & libc::PROT_WRITE as u64 != 0
        && outcome.enter.arg3 & libc::MAP_SHARED as u64 != 0
}

//! File-descriptor I/O decoding for non-socket targets.

use std::collections::BTreeMap;
use std::os::unix::fs::FileTypeExt;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::CollectorName;
use model_core::process::ProcessIdentity;

use crate::decode::FdIpcKind;
use crate::decode::FileTracker;
use crate::decode::{DecodeError, NET_EVENT_RECV, NET_EVENT_SEND};
use crate::loader::KernelObservationEvent;
use crate::maps::BindingStateMap;

const NET_SYSCALL_FD_IO_WRITEV: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FdTargetKind {
    Pipe,
    Fifo,
    UnixSocket,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FdObservation {
    kind: FdTargetKind,
    target: String,
    metadata: BTreeMap<String, String>,
}

pub(super) fn operation(kind: u32, syscall_family: u32) -> (&'static str, &'static str) {
    match (kind, syscall_family) {
        (NET_EVENT_SEND, NET_SYSCALL_FD_IO_WRITEV) => ("writev", "outbound"),
        (NET_EVENT_SEND, _) => ("write", "outbound"),
        (NET_EVENT_RECV, _) => ("read", "inbound"),
        _ => ("unknown", "unknown"),
    }
}

pub(super) fn decode(
    event: KernelObservationEvent,
    bindings: &BindingStateMap,
    identity: ProcessIdentity,
    operation: &'static str,
    direction: &'static str,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if let Some(kind) = file_tracker.resolve_fd_ipc_kind(event.trace_id, &identity, event.fd)
        && ipc_capability_enabled(kind.into(), bindings, event.trace_id)
    {
        let observation = FdObservation {
            kind: kind.into(),
            target: anonymous_ipc_target(kind, &event),
            metadata: BTreeMap::from([(
                "fd_target_source".to_string(),
                "ipc_fd_tracker".to_string(),
            )]),
        };
        let metadata = fd_io_metadata(&event, operation, direction, &observation);
        return Ok(Some(RawCollectorEvent {
            envelope: RawEventEnvelope {
                observed_at: SystemTime::now(),
                process: identity,
                collector: CollectorName::new("ebpf"),
            },
            payload: RawObservationPayload::Ipc {
                channel: fd_channel(observation.kind).to_string(),
                peer: Some(observation.target),
                metadata,
            },
        }));
    }
    if bindings.trace_has_capability(event.trace_id, &Capability::FsAccessBasic) {
        if let Some(path) = file_tracker.resolve_fd_path(event.trace_id, &identity, event.fd) {
            if let Some(kind) = tracked_path_ipc_kind(&path)
                && ipc_capability_enabled(kind, bindings, event.trace_id)
            {
                let observation = FdObservation {
                    kind,
                    target: path,
                    metadata: BTreeMap::from([(
                        "fd_target_source".to_string(),
                        "file_tracker".to_string(),
                    )]),
                };
                let metadata = fd_io_metadata(&event, operation, direction, &observation);
                return Ok(Some(RawCollectorEvent {
                    envelope: RawEventEnvelope {
                        observed_at: SystemTime::now(),
                        process: identity,
                        collector: CollectorName::new("ebpf"),
                    },
                    payload: RawObservationPayload::Ipc {
                        channel: fd_channel(observation.kind).to_string(),
                        peer: Some(observation.target),
                        metadata,
                    },
                }));
            }
            let metadata = tracked_file_metadata(&event, operation, direction, &path);
            return Ok(Some(RawCollectorEvent {
                envelope: RawEventEnvelope {
                    observed_at: SystemTime::now(),
                    process: identity,
                    collector: CollectorName::new("ebpf"),
                },
                payload: RawObservationPayload::File {
                    operation: operation.to_string(),
                    path: Some(path),
                    metadata,
                },
            }));
        }
    }
    Ok(None)
}

fn tracked_path_ipc_kind(path: &str) -> Option<FdTargetKind> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.file_type().is_fifo() {
        Some(FdTargetKind::Fifo)
    } else if metadata.file_type().is_socket() {
        Some(FdTargetKind::UnixSocket)
    } else {
        None
    }
}

fn fd_io_metadata(
    event: &KernelObservationEvent,
    operation: &str,
    direction: &str,
    observation: &FdObservation,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("direction".to_string(), direction.to_string()),
        ("fd".to_string(), event.fd.to_string()),
        ("result".to_string(), event.result.to_string()),
        ("fd_target".to_string(), observation.target.clone()),
        (
            "fd_target_kind".to_string(),
            fd_target_kind(observation.kind).to_string(),
        ),
    ]);
    if event.requested_size > 0 {
        metadata.insert(
            "requested_size".to_string(),
            event.requested_size.to_string(),
        );
    }
    if let Some(size) = fd_io_size(event.kind, event.result) {
        metadata.insert("size".to_string(), size.to_string());
    }
    metadata.extend(observation.metadata.clone());
    metadata
}

fn tracked_file_metadata(
    event: &KernelObservationEvent,
    operation: &str,
    direction: &str,
    path: &str,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("direction".to_string(), direction.to_string()),
        ("fd".to_string(), event.fd.to_string()),
        ("result".to_string(), event.result.to_string()),
        ("fd_target".to_string(), path.to_string()),
        ("fd_target_kind".to_string(), "regular_file".to_string()),
        ("fd_target_source".to_string(), "file_tracker".to_string()),
    ]);
    if event.requested_size > 0 {
        metadata.insert(
            "requested_size".to_string(),
            event.requested_size.to_string(),
        );
    }
    if let Some(size) = fd_io_size(event.kind, event.result) {
        metadata.insert("size".to_string(), size.to_string());
    }
    metadata
}

fn fd_io_size(kind: u32, result: i32) -> Option<u64> {
    if !matches!(kind, NET_EVENT_SEND | NET_EVENT_RECV) || result < 0 {
        return None;
    }
    Some(result as u64)
}

fn fd_channel(kind: FdTargetKind) -> &'static str {
    match kind {
        FdTargetKind::Pipe => "pipe",
        FdTargetKind::Fifo => "fifo",
        FdTargetKind::UnixSocket => "unix_socket",
    }
}

fn ipc_capability_enabled(
    kind: FdTargetKind,
    bindings: &BindingStateMap,
    trace_id: model_core::ids::TraceId,
) -> bool {
    match kind {
        FdTargetKind::Pipe | FdTargetKind::Fifo => {
            bindings.trace_has_capability(trace_id, &Capability::IpcPipeFifo)
        }
        FdTargetKind::UnixSocket => {
            bindings.trace_has_capability(trace_id, &Capability::IpcUnixSocket)
        }
    }
}

fn fd_target_kind(kind: FdTargetKind) -> &'static str {
    match kind {
        FdTargetKind::Pipe => "pipe",
        FdTargetKind::Fifo => "fifo",
        FdTargetKind::UnixSocket => "unix_socket",
    }
}

fn anonymous_ipc_target(kind: FdIpcKind, event: &KernelObservationEvent) -> String {
    let channel = fd_channel(kind.into());
    format!("{channel}:pid:{}:fd:{}", event.pid, event.fd)
}

impl From<FdIpcKind> for FdTargetKind {
    fn from(kind: FdIpcKind) -> Self {
        match kind {
            FdIpcKind::Pipe => Self::Pipe,
            FdIpcKind::UnixSocket => Self::UnixSocket,
        }
    }
}

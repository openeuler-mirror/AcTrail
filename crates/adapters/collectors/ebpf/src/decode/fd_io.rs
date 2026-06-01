//! File-descriptor I/O decoding for non-socket targets.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::CollectorName;
use model_core::process::ProcessIdentity;

use crate::decode::FileTracker;
use crate::decode::{DecodeError, NET_EVENT_RECV, NET_EVENT_SEND};
use crate::loader::KernelObservationEvent;
use crate::maps::BindingStateMap;
use crate::procfs::{FdObservation, FdTargetKind, resolve_fd_observation};

pub(super) fn operation(kind: u32) -> (&'static str, &'static str) {
    match kind {
        NET_EVENT_SEND => ("write", "outbound"),
        NET_EVENT_RECV => ("read", "inbound"),
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
    let observation = resolve_fd_observation(event.pid, event.fd)
        .map_err(|error| DecodeError::new("fd_observation", error))?;
    let Some(observation) = observation else {
        return decode_tracked_file(
            event,
            bindings,
            identity,
            operation,
            direction,
            file_tracker,
        );
    };
    let metadata = fd_io_metadata(&event, operation, direction, &observation);
    if ipc_capability_enabled(observation.kind, bindings, event.trace_id) {
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
    if observation.kind == FdTargetKind::RegularFile
        && bindings.trace_has_capability(event.trace_id, &Capability::FsAccessBasic)
    {
        return Ok(Some(RawCollectorEvent {
            envelope: RawEventEnvelope {
                observed_at: SystemTime::now(),
                process: identity,
                collector: CollectorName::new("ebpf"),
            },
            payload: RawObservationPayload::File {
                operation: operation.to_string(),
                path: Some(observation.target),
                metadata,
            },
        }));
    }
    Ok(None)
}

fn decode_tracked_file(
    event: KernelObservationEvent,
    bindings: &BindingStateMap,
    identity: ProcessIdentity,
    operation: &'static str,
    direction: &'static str,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if !bindings.trace_has_capability(event.trace_id, &Capability::FsAccessBasic) {
        return Ok(None);
    }
    let Some(path) = file_tracker.resolve_fd_path(event.pid, event.fd) else {
        return Ok(None);
    };
    let metadata = tracked_file_metadata(&event, operation, direction, &path);
    Ok(Some(RawCollectorEvent {
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
    }))
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
        FdTargetKind::Socket => "socket",
        FdTargetKind::RegularFile | FdTargetKind::Other => "fd",
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
        FdTargetKind::RegularFile | FdTargetKind::Socket | FdTargetKind::Other => false,
    }
}

fn fd_target_kind(kind: FdTargetKind) -> &'static str {
    match kind {
        FdTargetKind::RegularFile => "regular_file",
        FdTargetKind::Pipe => "pipe",
        FdTargetKind::Fifo => "fifo",
        FdTargetKind::UnixSocket => "unix_socket",
        FdTargetKind::Socket => "socket",
        FdTargetKind::Other => "other",
    }
}

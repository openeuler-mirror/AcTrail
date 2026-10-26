//! File-descriptor I/O decoding for non-socket targets.

use std::collections::BTreeMap;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::CollectorName;
use model_core::process::ProcessObservation;

use crate::decode::DecodeError;
use crate::decode::FileTracker;
use crate::loader::{KernelFdIoOperation, KernelFdIoPayload, KernelObservationCommon};
use crate::maps::BindingStateMap;

const SYSCALL_FAMILY_FD_IO_WRITEV: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FdTargetKind {
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

pub(super) fn operation(
    operation: KernelFdIoOperation,
    syscall_family: u32,
) -> (&'static str, &'static str) {
    match (operation, syscall_family) {
        (KernelFdIoOperation::Send, SYSCALL_FAMILY_FD_IO_WRITEV) => ("writev", "outbound"),
        (KernelFdIoOperation::Send, _) => ("write", "outbound"),
        (KernelFdIoOperation::Recv, _) => ("read", "inbound"),
    }
}

/// 无类别码事件仅使用文件路径事实。
pub(super) fn decode(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    bindings: &BindingStateMap,
    identity: ProcessObservation,
    operation: &'static str,
    direction: &'static str,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    decode_file(
        common,
        event,
        bindings,
        identity,
        operation,
        direction,
        file_tracker,
    )
}

/// 内核 fd_table 已判定为 IPC 时的直达分支。
pub(super) fn decode_ipc(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    bindings: &BindingStateMap,
    identity: ProcessObservation,
    operation: &'static str,
    direction: &'static str,
    kind: FdTargetKind,
    file_tracker: &FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if !ipc_capability_enabled(kind, bindings, common.trace_id) {
        return Ok(None);
    }
    let known_path = if kind == FdTargetKind::Fifo {
        file_tracker.resolve_fd_path(common.trace_id, &identity, event.fd)
    } else {
        None
    };
    let (target, source) = match known_path {
        Some(path) => (path, "file_tracker"),
        None => (ipc_fd_target(kind, common, event), "kernel_fd_table"),
    };
    let observation = FdObservation {
        kind,
        target,
        metadata: BTreeMap::from([("fd_target_source".to_string(), source.to_string())]),
    };
    Ok(Some(build_ipc_observation(
        common,
        event,
        identity,
        observation,
        operation,
        direction,
    )))
}

/// 内核按 inode 类型判定为 FILE 后，仅解析文件路径和文件观测字段。
pub(super) fn decode_file(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    bindings: &BindingStateMap,
    identity: ProcessObservation,
    operation: &'static str,
    direction: &'static str,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if !bindings.trace_has_capability(common.trace_id, &Capability::FsAccessBasic) {
        return Ok(None);
    }
    let Some(descriptor) =
        file_tracker.resolve_file_descriptor(common.trace_id, &identity, event.fd)
    else {
        return Ok(None);
    };
    let path = descriptor.path;
    let metadata =
        tracked_file_metadata(event, operation, direction, descriptor.creation_requested);
    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: super::clock::wall_from_ktime(common.observed_ktime_ns),
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

fn build_ipc_observation(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    identity: ProcessObservation,
    observation: FdObservation,
    operation: &str,
    direction: &str,
) -> RawCollectorEvent {
    let metadata = fd_io_metadata(event, operation, direction, &observation);
    RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: super::clock::wall_from_ktime(common.observed_ktime_ns),
            process: identity,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Ipc {
            channel: fd_channel(observation.kind).to_string(),
            peer: Some(observation.target),
            metadata,
        },
    }
}

fn fd_io_metadata(
    event: &KernelFdIoPayload,
    operation: &str,
    direction: &str,
    observation: &FdObservation,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("direction".to_string(), direction.to_string()),
        ("fd".to_string(), event.fd.to_string()),
        ("result".to_string(), event.syscall_result.to_string()),
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
    if let Some(size) = fd_io_size(event.operation, event.syscall_result) {
        metadata.insert("size".to_string(), size.to_string());
    }
    metadata.extend(observation.metadata.clone());
    metadata
}

fn tracked_file_metadata(
    event: &KernelFdIoPayload,
    operation: &str,
    direction: &str,
    creation_requested: bool,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("direction".to_string(), direction.to_string()),
        ("fd".to_string(), event.fd.to_string()),
        ("result".to_string(), event.syscall_result.to_string()),
        ("fd_target_kind".to_string(), "regular_file".to_string()),
        ("fd_target_source".to_string(), "file_tracker".to_string()),
    ]);
    if event.requested_size > 0 {
        metadata.insert(
            "requested_size".to_string(),
            event.requested_size.to_string(),
        );
    }
    if let Some(size) = fd_io_size(event.operation, event.syscall_result) {
        metadata.insert("size".to_string(), size.to_string());
    }
    if creation_requested {
        metadata.insert("fd_creation_requested".to_string(), "true".to_string());
    }
    metadata
}

fn fd_io_size(_operation: KernelFdIoOperation, result: i32) -> Option<u64> {
    if result < 0 {
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

// 仅作观测位置展示；跨进程的 MCP 通道身份由独立 stdio_bundle 提供。
fn ipc_fd_target(
    kind: FdTargetKind,
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
) -> String {
    let channel = fd_channel(kind);
    format!(
        "{channel}:pid:{}:fd:{}",
        common.subject.observer_namespace_tgid, event.fd
    )
}

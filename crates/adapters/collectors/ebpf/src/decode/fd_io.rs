//! File-descriptor I/O decoding for non-socket targets.

use std::collections::BTreeMap;
use std::os::unix::fs::FileTypeExt;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::capability::Capability;
use model_core::ids::CollectorName;
use model_core::process::ProcessObservation;

use crate::decode::DecodeError;
use crate::decode::FdIpcKind;
use crate::decode::FileTracker;
use crate::loader::{KernelFdIoOperation, KernelFdIoPayload, KernelObservationCommon};
use crate::maps::BindingStateMap;

const SYSCALL_FAMILY_FD_IO_WRITEV: u32 = 3;

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

/// 无类别码事件（fd 不在内核 fd_table）时的完整 fallback：先问 lineage 再问 path。
pub(super) fn decode(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    bindings: &BindingStateMap,
    identity: ProcessObservation,
    operation: &'static str,
    direction: &'static str,
    file_tracker: &mut FileTracker,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    if let Some(event) = decode_ipc(
        common,
        event,
        bindings,
        identity.clone(),
        operation,
        direction,
        file_tracker,
        None,
    )? {
        return Ok(Some(event));
    }
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

/// 内核 fd_table 已判定为 IPC（pipe/socketpair）时的直达分支：只查 lineage。
pub(super) fn decode_ipc(
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
    bindings: &BindingStateMap,
    identity: ProcessObservation,
    operation: &'static str,
    direction: &'static str,
    file_tracker: &mut FileTracker,
    classified_kind: Option<FdIpcKind>,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let tracked_kind = file_tracker.resolve_fd_ipc_kind(common.trace_id, &identity, event.fd);
    let Some(kind) = classified_kind.or(tracked_kind) else {
        return Ok(None);
    };
    if !ipc_capability_enabled(kind.into(), bindings, common.trace_id) {
        return Ok(None);
    }
    let observation = FdObservation {
        kind: kind.into(),
        target: anonymous_ipc_target(kind, common, event),
        metadata: BTreeMap::from([(
            "fd_target_source".to_string(),
            match classified_kind {
                Some(classified) if tracked_kind == Some(classified) => {
                    "kernel_fd_table+ipc_fd_tracker"
                }
                Some(_) => "kernel_fd_table",
                None => "ipc_fd_tracker",
            }
            .to_string(),
        )]),
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

/// 内核 fd_table 已判定为 FILE 时的直达分支：查 path 表；命名 fifo/socket 文件
/// 用一次 stat 回退补判（内核在 open 出口拿不到 inode 类型，只能登记成 FILE）。
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
    let Some(path) = file_tracker.resolve_fd_path(common.trace_id, &identity, event.fd) else {
        return Ok(None);
    };
    if let Some(kind) = tracked_path_ipc_kind(&path)
        && ipc_capability_enabled(kind, bindings, common.trace_id)
    {
        let observation = FdObservation {
            kind,
            target: path,
            metadata: BTreeMap::from([(
                "fd_target_source".to_string(),
                "file_tracker".to_string(),
            )]),
        };
        return Ok(Some(build_ipc_observation(
            common,
            event,
            identity,
            observation,
            operation,
            direction,
        )));
    }
    let creation_requested =
        file_tracker.fd_creation_requested(common.trace_id, &identity, event.fd);
    let metadata = tracked_file_metadata(event, operation, direction, &path, creation_requested);
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
    path: &str,
    creation_requested: bool,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("direction".to_string(), direction.to_string()),
        ("fd".to_string(), event.fd.to_string()),
        ("result".to_string(), event.syscall_result.to_string()),
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

fn anonymous_ipc_target(
    kind: FdIpcKind,
    common: &KernelObservationCommon,
    event: &KernelFdIoPayload,
) -> String {
    let channel = fd_channel(kind.into());
    format!(
        "{channel}:pid:{}:fd:{}",
        common.subject.observer_namespace_tgid, event.fd
    )
}

impl From<FdIpcKind> for FdTargetKind {
    fn from(kind: FdIpcKind) -> Self {
        match kind {
            FdIpcKind::Pipe => Self::Pipe,
            FdIpcKind::UnixSocket => Self::UnixSocket,
        }
    }
}

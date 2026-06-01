//! Seccomp user-notify socket payload capture.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use config_core::daemon::{PayloadSocketCaptureBackend, PayloadSocketConfig};
use control_contract::reply::ControlError;
use ebpf_collector::{
    EbpfCollector, SOCKET_PAYLOAD_DIRECTION_OUTBOUND, SOCKET_PAYLOAD_SYSCALL_SENDTO,
    SOCKET_PAYLOAD_SYSCALL_WRITE, SocketPayloadCompletion,
};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use trace_runtime::registry::TraceRuntime;

use crate::services::payload_gate::socket_payload_prefix_is_http_candidate;
use crate::services::seccomp_notify::read_linear_payload;

#[derive(Debug)]
pub(crate) struct SeccompSocketService {
    enabled: bool,
    max_operation_bytes: u32,
    max_segment_bytes: u32,
    http_sniff_max_bytes: u64,
    max_pending_operations: u32,
    captures: BTreeMap<SocketCaptureKey, CapturedSocketOperation>,
}

impl SeccompSocketService {
    pub(crate) fn new(config: &PayloadSocketConfig) -> Self {
        Self {
            enabled: config.enabled
                && config.capture_backend == PayloadSocketCaptureBackend::BpfCopySeccompFallback,
            max_operation_bytes: config.max_operation_bytes,
            max_segment_bytes: config.max_segment_bytes,
            http_sniff_max_bytes: config.http_sniff_max_bytes,
            max_pending_operations: config.pending_operation_max_entries,
            captures: BTreeMap::new(),
        }
    }

    pub(crate) fn handle_notification(
        &mut self,
        collector: &EbpfCollector,
        trace_runtime: &TraceRuntime,
        notification: &libc::seccomp_notif,
    ) -> Result<(), ControlError> {
        if !self.enabled {
            return Ok(());
        }
        if collector
            .lookup_pending_tls_payload_op(notification.pid)
            .map_err(|error| ControlError::new(error.stage, error.message))?
            .is_some()
        {
            return Ok(());
        }
        let Some(request) = SocketReadRequest::from_notification(notification)? else {
            return Ok(());
        };
        if request.requested_size <= u64::from(self.max_segment_bytes) {
            return Ok(());
        }
        if request.syscall == SOCKET_PAYLOAD_SYSCALL_WRITE
            && !fd_is_socket(notification.pid, request.fd)?
        {
            return Ok(());
        }
        let tgid = tgid_from_status(notification.pid)?;
        let Some((trace_id, membership)) = trace_runtime.find_membership_by_pid(tgid) else {
            return Ok(());
        };
        if !membership.capture_enabled {
            return Ok(());
        }
        let prefix_size = self
            .http_sniff_max_bytes
            .min(u64::from(self.max_operation_bytes))
            .min(request.requested_size);
        let prefix = read_linear_payload(
            tgid,
            request.buffer_ptr,
            prefix_size,
            self.max_operation_bytes,
        )?;
        let reached_sniff_limit =
            prefix.len() as u64 >= self.http_sniff_max_bytes.min(request.requested_size);
        if !socket_payload_prefix_is_http_candidate(&prefix, reached_sniff_limit) {
            return Ok(());
        }

        let bytes = read_linear_payload(
            tgid,
            request.buffer_ptr,
            request.requested_size,
            self.max_operation_bytes,
        )?;
        self.captures.insert(
            SocketCaptureKey {
                pid: tgid,
                fd: request.fd,
                syscall: request.syscall,
                buffer_ptr: request.buffer_ptr,
                requested_size: request.requested_size,
            },
            CapturedSocketOperation {
                trace_id,
                process: membership.identity,
                bytes,
            },
        );
        self.ensure_pending_capacity()
    }

    pub(crate) fn complete_operations(
        &mut self,
        completions: Vec<SocketPayloadCompletion>,
    ) -> Result<Vec<RawPayloadSegment>, ControlError> {
        let mut segments = Vec::new();
        for completion in completions {
            if completion.direction != SOCKET_PAYLOAD_DIRECTION_OUTBOUND {
                continue;
            }
            let Some(capture) = self.captures.remove(&SocketCaptureKey {
                pid: completion.pid,
                fd: completion.fd,
                syscall: completion.syscall,
                buffer_ptr: completion.buffer_ptr,
                requested_size: completion.requested_size,
            }) else {
                continue;
            };
            let operation_original_size = completion.completed_size;
            let operation_captured_size = (capture.bytes.len() as u64).min(operation_original_size);
            let captured_len = usize::try_from(operation_captured_size).map_err(|error| {
                ControlError::new(
                    "seccomp_socket_segment",
                    format!("captured size overflow: {error}"),
                )
            })?;
            let completion_state = if operation_captured_size == operation_original_size {
                PayloadOperationCompletionState::Success
            } else {
                PayloadOperationCompletionState::Partial
            };
            let truncation = if operation_captured_size == operation_original_size {
                PayloadTruncationState::Complete
            } else {
                PayloadTruncationState::Truncated
            };
            segments.push(RawPayloadSegment {
                trace_id: capture.trace_id,
                observed_at: SystemTime::now(),
                process: capture.process,
                source_boundary: PayloadSourceBoundary::Syscall,
                content_state: PayloadContentState::Plaintext,
                direction: PayloadDirection::Outbound,
                stream_key: PayloadStreamKey::new(format!(
                    "socket:{}:{}:{}",
                    completion.pid, completion.fd, completion.fd_generation
                )),
                sequence: completion.sequence,
                original_size: operation_original_size,
                captured_size: operation_captured_size,
                operation_id: completion.sequence,
                operation_offset: 0,
                operation_original_size,
                operation_captured_size,
                operation_completion_state: completion_state,
                truncation,
                library: "socket-syscall".to_string(),
                symbol: socket_symbol(completion.syscall)?.to_string(),
                protocol_hint: None,
                bytes: capture.bytes[..captured_len].to_vec(),
            });
        }
        self.ensure_pending_capacity()?;
        Ok(segments)
    }

    fn ensure_pending_capacity(&self) -> Result<(), ControlError> {
        let limit = usize::try_from(self.max_pending_operations).map_err(|error| {
            ControlError::new(
                "seccomp_socket_pending",
                format!("pending operation limit overflow: {error}"),
            )
        })?;
        if self.captures.len() > limit {
            return Err(ControlError::new(
                "seccomp_socket_pending",
                format!(
                    "pending socket operations {} exceed configured limit {limit}",
                    self.captures.len()
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct CapturedSocketOperation {
    trace_id: model_core::ids::TraceId,
    process: ProcessIdentity,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SocketCaptureKey {
    pid: u32,
    fd: u32,
    syscall: u32,
    buffer_ptr: u64,
    requested_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SocketReadRequest {
    fd: u32,
    syscall: u32,
    buffer_ptr: u64,
    requested_size: u64,
}

impl SocketReadRequest {
    fn from_notification(notification: &libc::seccomp_notif) -> Result<Option<Self>, ControlError> {
        let syscall = syscall_from_notification(notification)?;
        let Some(syscall) = syscall else {
            return Ok(None);
        };
        Ok(Some(Self {
            fd: u32::try_from(notification.data.args[0]).map_err(|error| {
                ControlError::new("seccomp_socket_args", format!("fd overflow: {error}"))
            })?,
            syscall,
            buffer_ptr: notification.data.args[1],
            requested_size: notification.data.args[2],
        }))
    }
}

fn syscall_from_notification(
    notification: &libc::seccomp_notif,
) -> Result<Option<u32>, ControlError> {
    let raw = i64::from(notification.data.nr);
    if raw == libc::SYS_write {
        return Ok(Some(SOCKET_PAYLOAD_SYSCALL_WRITE));
    }
    if raw == libc::SYS_sendto {
        return Ok(Some(SOCKET_PAYLOAD_SYSCALL_SENDTO));
    }
    Ok(None)
}

fn fd_is_socket(pid: u32, fd: u32) -> Result<bool, ControlError> {
    let path = PathBuf::from(format!("/proc/{pid}/fd/{fd}"));
    let target = std::fs::read_link(&path).map_err(|error| {
        ControlError::new(
            "seccomp_socket_fd",
            format!("readlink {}: {error}", path.display()),
        )
    })?;
    Ok(target.to_string_lossy().starts_with("socket:["))
}

fn tgid_from_status(tid: u32) -> Result<u32, ControlError> {
    let status = std::fs::read_to_string(format!("/proc/{tid}/status"))
        .map_err(|error| ControlError::new("seccomp_socket_tgid", error.to_string()))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Tgid:"))
        .map(str::trim)
        .ok_or_else(|| ControlError::new("seccomp_socket_tgid", "missing Tgid"))?
        .parse::<u32>()
        .map_err(|error| ControlError::new("seccomp_socket_tgid", error.to_string()))
}

fn socket_symbol(syscall: u32) -> Result<&'static str, ControlError> {
    match syscall {
        SOCKET_PAYLOAD_SYSCALL_WRITE => Ok("write"),
        SOCKET_PAYLOAD_SYSCALL_SENDTO => Ok("sendto"),
        other => Err(ControlError::new(
            "seccomp_socket_symbol",
            format!("unsupported socket payload syscall {other}"),
        )),
    }
}

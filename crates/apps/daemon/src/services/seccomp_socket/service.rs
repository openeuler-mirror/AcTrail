//! Seccomp user-notify socket payload capture.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{PayloadSocketCaptureBackend, PayloadSocketConfig};
use control_contract::reply::ControlError;
use ebpf_collector::{EbpfCollector, SOCKET_PAYLOAD_DIRECTION_OUTBOUND, SocketPayloadCompletion};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessObservation;
use payload_event::RawPayloadSegment;
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

use super::http::{HTTP1_PROTOCOL_HINT, content_length_admission};
use super::request::{SocketReadRequest, fd_is_socket, socket_symbol, tgid_from_status};
use crate::services::payload_gate::socket_payload_prefix_is_http_candidate;

#[derive(Debug)]
pub(crate) struct SeccompSocketService {
    enabled: bool,
    max_operation_bytes: u32,
    max_segment_bytes: u32,
    http_sniff_max_bytes: u64,
    max_pending_operations: u32,
    max_stream_states: u32,
    captures: BTreeMap<SocketCaptureKey, CapturedSocketOperation>,
    continuations: BTreeMap<SocketContinuationKey, HttpBodyContinuation>,
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
            max_stream_states: config.stream_state_max_entries,
            captures: BTreeMap::new(),
            continuations: BTreeMap::new(),
        }
    }

    pub(crate) fn handle_notification(
        &mut self,
        collector: &EbpfCollector,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
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
        if request.requires_socket_fd_check() && !fd_is_socket(notification.pid, request.fd)? {
            return Ok(());
        }
        let Some(tgid) = tgid_from_status(notification.pid)? else {
            return Ok(());
        };
        let Some(identity) = process_registry.active_host_pid(tgid) else {
            return Ok(());
        };
        let Some((trace_id, membership)) = trace_runtime.find_membership(&identity) else {
            return Ok(());
        };
        if !membership.capture_enabled {
            return Ok(());
        }
        let fd_generation = collector
            .lookup_socket_fd_generation(tgid, request.fd)
            .map_err(|error| ControlError::new(error.stage, error.message))?
            .unwrap_or(0);
        let stream_key = SocketContinuationKey {
            trace_id: trace_id.get(),
            pid: tgid,
            fd: request.fd,
            fd_generation,
            direction: SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
        };
        let prefix_size = self
            .http_sniff_max_bytes
            .min(u64::from(self.max_operation_bytes))
            .min(request.read_size_hint());
        let Some(prefix) = request.read_payload(tgid, prefix_size, self.max_operation_bytes)?
        else {
            return Ok(());
        };
        let reached_sniff_limit =
            prefix.len() as u64 >= self.http_sniff_max_bytes.min(request.read_size_hint());
        if request.skip_small_linear_payload(self.max_segment_bytes) {
            if fd_generation != 0 {
                self.record_small_http_prefix(&stream_key, &prefix)?;
            }
            return self.ensure_capacity();
        }

        let http_candidate = socket_payload_prefix_is_http_candidate(&prefix, reached_sniff_limit);
        let capture_update = if http_candidate {
            self.http_message_capture_update(&stream_key, fd_generation, &prefix)
        } else {
            let Some(continuation) = self.continuation_capture_update(&stream_key) else {
                return Ok(());
            };
            continuation
        };
        let read_limit = capture_update
            .read_limit()
            .unwrap_or(u64::from(self.max_operation_bytes));

        let Some(bytes) = request.read_payload(tgid, read_limit, self.max_operation_bytes)? else {
            return Ok(());
        };
        self.captures.insert(
            SocketCaptureKey {
                pid: tgid,
                fd: request.fd,
                syscall: request.syscall,
                buffer_ptr: request.key_buffer_ptr,
                requested_size: request.key_requested_size,
            },
            CapturedSocketOperation {
                trace_id,
                process: process_registry
                    .record(membership.identity)
                    .ok_or_else(|| {
                        ControlError::new("seccomp_socket", "process record is missing")
                    })?
                    .observation(),
                bytes,
                protocol_hint: capture_update.protocol_hint(),
                update: capture_update,
            },
        );
        self.ensure_capacity()
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
            let captured_bytes = &capture.bytes[..captured_len];
            let segment_max = usize::try_from(self.max_segment_bytes).map_err(|error| {
                ControlError::new(
                    "seccomp_socket_segment",
                    format!("segment size overflow: {error}"),
                )
            })?;
            if segment_max == 0 {
                return Err(ControlError::new(
                    "seccomp_socket_segment",
                    "payload_socket_max_segment_bytes must be positive",
                ));
            }
            for (index, chunk) in captured_bytes.chunks(segment_max).enumerate() {
                let offset = index.checked_mul(segment_max).ok_or_else(|| {
                    ControlError::new("seccomp_socket_segment", "offset overflow")
                })?;
                let final_chunk = offset + chunk.len() >= captured_bytes.len();
                let truncation = if final_chunk && operation_captured_size < operation_original_size
                {
                    PayloadTruncationState::Truncated
                } else {
                    PayloadTruncationState::Complete
                };
                segments.push(RawPayloadSegment {
                    trace_id: capture.trace_id,
                    observed_at: SystemTime::now(),
                    process: capture.process.clone(),
                    source_boundary: PayloadSourceBoundary::Syscall,
                    content_state: PayloadContentState::Plaintext,
                    direction: PayloadDirection::Outbound,
                    stream_key: PayloadStreamKey::new(format!(
                        "socket:{}:{}:{}",
                        completion.pid, completion.fd, completion.fd_generation
                    )),
                    sequence: completion.sequence + index as u64,
                    original_size: if truncation == PayloadTruncationState::Truncated {
                        operation_original_size.saturating_sub(offset as u64)
                    } else {
                        chunk.len() as u64
                    },
                    captured_size: chunk.len() as u64,
                    operation_id: completion.sequence,
                    operation_offset: offset as u64,
                    operation_original_size,
                    operation_captured_size,
                    operation_completion_state: completion_state,
                    truncation,
                    library: "socket-syscall".to_string(),
                    symbol: socket_symbol(completion.syscall)?.to_string(),
                    protocol_hint: capture.protocol_hint.clone(),
                    bytes: chunk.to_vec(),
                });
            }
            self.apply_capture_update(
                capture.update,
                operation_original_size,
                operation_captured_size,
                captured_len as u64,
            )?;
        }
        self.ensure_capacity()?;
        Ok(segments)
    }

    pub(crate) fn forget_trace(&mut self, trace_id: TraceId) {
        self.continuations
            .retain(|key, _| key.trace_id != trace_id.get());
    }

    fn record_small_http_prefix(
        &mut self,
        stream_key: &SocketContinuationKey,
        prefix: &[u8],
    ) -> Result<(), ControlError> {
        let Some(admission) = content_length_admission(prefix) else {
            return Ok(());
        };
        let remaining = admission
            .content_length
            .saturating_sub(admission.body_bytes_in_buffer);
        if remaining == 0 {
            self.continuations.remove(stream_key);
            return Ok(());
        }
        let budget = u64::from(self.max_operation_bytes)
            .saturating_sub(admission.body_bytes_in_buffer)
            .min(remaining);
        if budget == 0 {
            self.continuations.remove(stream_key);
            return Ok(());
        }
        self.continuations.insert(
            stream_key.clone(),
            HttpBodyContinuation {
                remaining_body_bytes: remaining,
                remaining_capture_budget_bytes: budget,
            },
        );
        self.ensure_stream_capacity()
    }

    fn http_message_capture_update(
        &self,
        stream_key: &SocketContinuationKey,
        fd_generation: u32,
        prefix: &[u8],
    ) -> SocketCaptureUpdate {
        if fd_generation == 0 {
            return SocketCaptureUpdate::None;
        }
        let Some(admission) = content_length_admission(prefix) else {
            return SocketCaptureUpdate::None;
        };
        SocketCaptureUpdate::HttpMessage {
            stream_key: stream_key.clone(),
            content_length: admission.content_length,
            header_len: admission.header_len,
        }
    }

    fn continuation_capture_update(
        &self,
        stream_key: &SocketContinuationKey,
    ) -> Option<SocketCaptureUpdate> {
        let continuation = self.continuations.get(stream_key)?;
        if continuation.remaining_body_bytes == 0
            || continuation.remaining_capture_budget_bytes == 0
        {
            return None;
        }
        Some(SocketCaptureUpdate::Continuation {
            stream_key: stream_key.clone(),
            read_limit: continuation
                .remaining_body_bytes
                .min(continuation.remaining_capture_budget_bytes),
        })
    }

    fn apply_capture_update(
        &mut self,
        update: SocketCaptureUpdate,
        operation_original_size: u64,
        operation_captured_size: u64,
        captured_len: u64,
    ) -> Result<(), ControlError> {
        match update {
            SocketCaptureUpdate::None => {}
            SocketCaptureUpdate::HttpMessage {
                stream_key,
                content_length,
                header_len,
            } => {
                if operation_original_size != operation_captured_size {
                    self.continuations.remove(&stream_key);
                    return Ok(());
                }
                let body_bytes = operation_original_size
                    .saturating_sub(header_len)
                    .min(content_length);
                self.record_remaining_body(stream_key, content_length, body_bytes)?;
            }
            SocketCaptureUpdate::Continuation { stream_key, .. } => {
                self.apply_continuation_progress(stream_key, captured_len);
            }
        }
        Ok(())
    }

    fn record_remaining_body(
        &mut self,
        stream_key: SocketContinuationKey,
        content_length: u64,
        body_bytes_completed: u64,
    ) -> Result<(), ControlError> {
        let remaining = content_length.saturating_sub(body_bytes_completed);
        if remaining == 0 {
            self.continuations.remove(&stream_key);
            return Ok(());
        }
        let budget = u64::from(self.max_operation_bytes)
            .saturating_sub(body_bytes_completed)
            .min(remaining);
        if budget == 0 {
            self.continuations.remove(&stream_key);
            return Ok(());
        }
        self.continuations.insert(
            stream_key,
            HttpBodyContinuation {
                remaining_body_bytes: remaining,
                remaining_capture_budget_bytes: budget,
            },
        );
        self.ensure_stream_capacity()
    }

    fn apply_continuation_progress(
        &mut self,
        stream_key: SocketContinuationKey,
        captured_len: u64,
    ) {
        let Some(continuation) = self.continuations.get_mut(&stream_key) else {
            return;
        };
        continuation.remaining_body_bytes = continuation
            .remaining_body_bytes
            .saturating_sub(captured_len);
        continuation.remaining_capture_budget_bytes = continuation
            .remaining_capture_budget_bytes
            .saturating_sub(captured_len);
        if continuation.remaining_body_bytes == 0
            || continuation.remaining_capture_budget_bytes == 0
        {
            self.continuations.remove(&stream_key);
        }
    }

    fn ensure_capacity(&self) -> Result<(), ControlError> {
        self.ensure_pending_capacity()?;
        self.ensure_stream_capacity()
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

    fn ensure_stream_capacity(&self) -> Result<(), ControlError> {
        let limit = usize::try_from(self.max_stream_states).map_err(|error| {
            ControlError::new(
                "seccomp_socket_stream_state",
                format!("stream state limit overflow: {error}"),
            )
        })?;
        if self.continuations.len() > limit {
            return Err(ControlError::new(
                "seccomp_socket_stream_state",
                format!(
                    "socket HTTP continuation streams {} exceed configured limit {limit}",
                    self.continuations.len()
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct CapturedSocketOperation {
    trace_id: TraceId,
    process: ProcessObservation,
    bytes: Vec<u8>,
    protocol_hint: Option<String>,
    update: SocketCaptureUpdate,
}

#[derive(Clone, Debug)]
enum SocketCaptureUpdate {
    None,
    HttpMessage {
        stream_key: SocketContinuationKey,
        content_length: u64,
        header_len: u64,
    },
    Continuation {
        stream_key: SocketContinuationKey,
        read_limit: u64,
    },
}

impl SocketCaptureUpdate {
    fn read_limit(&self) -> Option<u64> {
        match self {
            Self::None | Self::HttpMessage { .. } => None,
            Self::Continuation { read_limit, .. } => Some(*read_limit),
        }
    }

    fn protocol_hint(&self) -> Option<String> {
        match self {
            Self::Continuation { .. } => Some(HTTP1_PROTOCOL_HINT.to_string()),
            Self::None | Self::HttpMessage { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SocketContinuationKey {
    trace_id: u64,
    pid: u32,
    fd: u32,
    fd_generation: u32,
    direction: u32,
}

#[derive(Clone, Debug)]
struct HttpBodyContinuation {
    remaining_body_bytes: u64,
    remaining_capture_budget_bytes: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SocketCaptureKey {
    pid: u32,
    fd: u32,
    syscall: u32,
    buffer_ptr: u64,
    requested_size: u64,
}

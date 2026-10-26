//! Seccomp user-notify socket payload capture.

use std::collections::{BTreeMap, VecDeque};

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

use super::identity::SocketCaptureKey;
use super::request::{SocketReadRequest, fd_is_socket, socket_symbol, tgid_from_status};
use crate::services::payload_gate::socket_payload_prefix_is_tls_hello;

#[derive(Debug)]
pub(crate) struct SeccompSocketService {
    enabled: bool,
    max_operation_bytes: u32,
    max_segment_bytes: u32,
    max_pending_operations: u32,
    captures: BTreeMap<SocketCaptureKey, VecDeque<CapturedSocketOperation>>,
}

impl SeccompSocketService {
    pub(crate) fn new(config: &PayloadSocketConfig) -> Self {
        Self {
            enabled: config.enabled
                && config.capture_backend == PayloadSocketCaptureBackend::BpfCopySeccompFallback,
            max_operation_bytes: config.max_operation_bytes,
            max_segment_bytes: config.max_segment_bytes,
            max_pending_operations: config.pending_operation_max_entries,
            captures: BTreeMap::new(),
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
        let Some((kernel_tgid, pid_generation)) = collector.socket_capture_process(tgid, trace_id)
        else {
            return Ok(());
        };
        let Some(fd_state) = collector
            .lookup_socket_fd_state(kernel_tgid, request.fd)
            .map_err(|error| ControlError::new(error.stage, error.message))?
        else {
            return Ok(());
        };
        if fd_state.tls_owned() {
            return Ok(());
        }
        if request.skip_small_linear_payload(self.max_segment_bytes) {
            return Ok(());
        }
        // Admit before allocating plaintext; the queue is bounded independently
        // of HTTP/WebSocket classification and owns only pending syscall bytes.
        self.ensure_pending_capacity()?;
        let capture_key = SocketCaptureKey::from_request(
            trace_id,
            pid_generation,
            fd_state.socket_generation(),
            kernel_tgid,
            notification.pid,
            &request,
        );
        let Some(bytes) = request.read_payload(
            tgid,
            u64::from(self.max_operation_bytes),
            self.max_operation_bytes,
        )?
        else {
            return Ok(());
        };
        if socket_payload_prefix_is_tls_hello(&bytes) {
            collector
                .mark_socket_fd_tls_owned(kernel_tgid, request.fd, fd_state.socket_generation())
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            return Ok(());
        }
        self.queue_capture(
            capture_key,
            CapturedSocketOperation {
                trace_id,
                process: process_registry
                    .record(membership.identity)
                    .ok_or_else(|| {
                        ControlError::new("seccomp_socket", "process record is missing")
                    })?
                    .observation(),
                bytes,
            },
        );
        Ok(())
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
            let Some(capture) = self.take_capture(&SocketCaptureKey::from_completion(&completion))
            else {
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
                    observed_at: completion.observed_at,
                    process: capture.process.clone(),
                    source_boundary: PayloadSourceBoundary::Syscall,
                    content_state: PayloadContentState::Plaintext,
                    direction: PayloadDirection::Outbound,
                    stream_key: PayloadStreamKey::new(format!(
                        "socket:{}:{}:{}",
                        completion.kernel_tgid, completion.fd, completion.socket_generation
                    )),
                    sequence: completion.sequence,
                    original_size: if truncation == PayloadTruncationState::Truncated {
                        operation_original_size.saturating_sub(offset as u64)
                    } else {
                        chunk.len() as u64
                    },
                    captured_size: chunk.len() as u64,
                    operation_id: completion.operation_id,
                    operation_chunk_index: index as u32,
                    operation_offset: offset as u64,
                    operation_original_size,
                    operation_captured_size,
                    operation_completion_state: completion_state,
                    truncation,
                    library: "socket-syscall".to_string(),
                    symbol: socket_symbol(completion.syscall)?.to_string(),
                    protocol_hint: None,
                    bytes: chunk.to_vec(),
                });
            }
        }
        Ok(segments)
    }

    pub(crate) fn forget_trace(&mut self, trace_id: TraceId) {
        self.captures.retain(|key, _| key.trace_id != trace_id);
    }

    fn take_capture(&mut self, key: &SocketCaptureKey) -> Option<CapturedSocketOperation> {
        let queue = self.captures.get_mut(key)?;
        let capture = queue.pop_front();
        let empty = queue.is_empty();
        if empty {
            self.captures.remove(key);
        }
        capture
    }

    fn queue_capture(&mut self, key: SocketCaptureKey, capture: CapturedSocketOperation) {
        self.captures.entry(key).or_default().push_back(capture);
    }

    fn ensure_pending_capacity(&self) -> Result<(), ControlError> {
        let limit = usize::try_from(self.max_pending_operations).map_err(|error| {
            ControlError::new(
                "seccomp_socket_pending",
                format!("pending operation limit overflow: {error}"),
            )
        })?;
        let pending = self
            .captures
            .values()
            .fold(0usize, |count, queue| count.saturating_add(queue.len()));
        if pending >= limit {
            return Err(ControlError::new(
                "seccomp_socket_pending",
                format!(
                    "pending socket operations {} reach configured limit {limit}",
                    pending
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
}

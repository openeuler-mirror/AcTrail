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

use super::http::{HTTP1_PROTOCOL_HINT, content_length_admission};
use super::request::{SocketReadRequest, fd_is_socket, socket_symbol, tgid_from_status};
use crate::services::payload_gate::{
    socket_payload_prefix_is_http_candidate, socket_payload_prefix_is_tls_hello,
};

#[derive(Debug)]
pub(crate) struct SeccompSocketService {
    enabled: bool,
    max_operation_bytes: u32,
    max_segment_bytes: u32,
    http_sniff_max_bytes: u64,
    max_pending_operations: u32,
    max_stream_states: u32,
    captures: BTreeMap<SocketCaptureKey, VecDeque<CapturedSocketOperation>>,
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
        let fd_state = collector
            .lookup_socket_fd_state(tgid, request.fd)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        if fd_state.is_some_and(|state| state.tls_owned()) {
            return Ok(());
        }
        let fd_generation = fd_state.map_or(0, |state| state.generation());
        let stream_key = SocketContinuationKey {
            trace_id: trace_id.get(),
            pid: tgid,
            fd: request.fd,
            fd_generation,
            direction: SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
        };
        let capture_key = SocketCaptureKey::from_request(tgid, notification.pid, &request);
        let linear_payload_range = request.linear_payload_range();
        let prefix_size = self
            .http_sniff_max_bytes
            .min(u64::from(self.max_operation_bytes))
            .min(request.read_size_hint());
        let Some(prefix) = request.read_payload(tgid, prefix_size, self.max_operation_bytes)?
        else {
            return Ok(());
        };
        if socket_payload_prefix_is_tls_hello(&prefix) {
            if let Some(fd_state) = fd_state {
                collector
                    .mark_socket_fd_tls_owned(tgid, request.fd, fd_state.generation())
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                self.continuations.remove(&stream_key);
            }
            return Ok(());
        }
        let reached_sniff_limit =
            prefix.len() as u64 >= self.http_sniff_max_bytes.min(request.read_size_hint());
        if request.skip_small_linear_payload(self.max_segment_bytes) {
            if fd_generation != 0 {
                self.record_small_http_prefix(
                    &stream_key,
                    &capture_key,
                    linear_payload_range,
                    &prefix,
                )?;
            }
            return self.ensure_capacity();
        }

        let http_candidate = socket_payload_prefix_is_http_candidate(&prefix, reached_sniff_limit);
        let capture_update = if http_candidate {
            self.http_message_capture_update(
                &stream_key,
                fd_generation,
                &capture_key,
                linear_payload_range,
                &prefix,
            )
        } else {
            let Some(continuation) =
                self.continuation_capture_update(&stream_key, linear_payload_range)
            else {
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
        // The listener can receive a partial-write retry before the collector polls the first
        // syscall's completion. Arm the HTTP continuation while the target is still blocked.
        self.prepare_capture_update(&capture_update);
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
                        completion.pid, completion.fd, completion.fd_generation
                    )),
                    sequence: completion.sequence,
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
        message_key: &SocketCaptureKey,
        linear_payload_range: Option<(u64, u64)>,
        prefix: &[u8],
    ) -> Result<(), ControlError> {
        let Some(admission) = content_length_admission(prefix) else {
            return Ok(());
        };
        let capture_body_limit = admission
            .content_length
            .min(u64::from(self.max_operation_bytes));
        let confirmed_body_end = admission.body_bytes_in_buffer.min(capture_body_limit);
        if confirmed_body_end >= capture_body_limit {
            self.continuations.remove(stream_key);
            return Ok(());
        }
        self.continuations.insert(
            stream_key.clone(),
            HttpBodyContinuation {
                message_key: message_key.clone(),
                content_length: admission.content_length,
                header_len: admission.header_len,
                linear_payload_range,
                confirmed_body_end,
            },
        );
        self.ensure_stream_capacity()
    }

    fn http_message_capture_update(
        &self,
        stream_key: &SocketContinuationKey,
        fd_generation: u32,
        message_key: &SocketCaptureKey,
        linear_payload_range: Option<(u64, u64)>,
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
            message_key: message_key.clone(),
            content_length: admission.content_length,
            header_len: admission.header_len,
            linear_payload_range,
        }
    }

    fn continuation_capture_update(
        &self,
        stream_key: &SocketContinuationKey,
        linear_payload_range: Option<(u64, u64)>,
    ) -> Option<SocketCaptureUpdate> {
        let continuation = self.continuations.get(stream_key)?;
        let capture_body_limit = continuation.capture_body_limit(self.max_operation_bytes);
        if continuation.confirmed_body_end >= capture_body_limit {
            return None;
        }

        let (read_limit, body_offset, header_bytes_remaining) =
            match linear_suffix_offset(continuation.linear_payload_range, linear_payload_range) {
                Some(raw_offset) => {
                    // A buffer contained inside the original linear write proves where this
                    // retry belongs even while the initial completion is still pending.
                    let raw_capture_end =
                        continuation.header_len.checked_add(capture_body_limit)?;
                    if raw_offset >= raw_capture_end {
                        return None;
                    }
                    (
                        raw_capture_end.saturating_sub(raw_offset),
                        Some(raw_offset.saturating_sub(continuation.header_len)),
                        continuation.header_len.saturating_sub(raw_offset),
                    )
                }
                None => (
                    // TCP preserves byte order. While Content-Length is still outstanding, the
                    // next non-HTTP write on this fd is body continuation even when the runtime
                    // rebuilt the buffer or changed from a linear syscall to writev/sendmsg.
                    capture_body_limit.saturating_sub(continuation.confirmed_body_end),
                    None,
                    0,
                ),
            };
        if read_limit == 0 {
            return None;
        }
        Some(SocketCaptureUpdate::Continuation {
            stream_key: stream_key.clone(),
            message_key: continuation.message_key.clone(),
            read_limit,
            body_offset,
            header_bytes_remaining,
        })
    }

    fn prepare_capture_update(&mut self, update: &SocketCaptureUpdate) {
        let SocketCaptureUpdate::HttpMessage {
            stream_key,
            message_key,
            content_length,
            header_len,
            linear_payload_range,
        } = update
        else {
            return;
        };
        self.continuations.insert(
            stream_key.clone(),
            HttpBodyContinuation {
                message_key: message_key.clone(),
                content_length: *content_length,
                header_len: *header_len,
                linear_payload_range: *linear_payload_range,
                confirmed_body_end: 0,
            },
        );
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
                message_key,
                content_length,
                header_len,
                ..
            } => {
                if operation_original_size != operation_captured_size {
                    self.remove_matching_continuation(&stream_key, &message_key);
                    return Ok(());
                }
                self.apply_initial_completion(
                    &stream_key,
                    &message_key,
                    content_length,
                    header_len,
                    operation_original_size,
                );
            }
            SocketCaptureUpdate::Continuation {
                stream_key,
                message_key,
                body_offset,
                header_bytes_remaining,
                ..
            } => {
                self.apply_continuation_progress(
                    &stream_key,
                    &message_key,
                    body_offset,
                    header_bytes_remaining,
                    captured_len,
                );
            }
        }
        Ok(())
    }

    fn apply_initial_completion(
        &mut self,
        stream_key: &SocketContinuationKey,
        message_key: &SocketCaptureKey,
        content_length: u64,
        header_len: u64,
        completed_size: u64,
    ) {
        let max_operation_bytes = self.max_operation_bytes;
        let Some(continuation) = self.continuations.get_mut(stream_key) else {
            return;
        };
        if continuation.message_key != *message_key {
            return;
        }
        let capture_body_limit = content_length.min(u64::from(max_operation_bytes));
        let body_end = completed_size
            .saturating_sub(header_len)
            .min(capture_body_limit);
        continuation.confirmed_body_end = continuation.confirmed_body_end.max(body_end);
        if continuation.confirmed_body_end >= capture_body_limit {
            self.continuations.remove(stream_key);
        }
    }

    fn apply_continuation_progress(
        &mut self,
        stream_key: &SocketContinuationKey,
        message_key: &SocketCaptureKey,
        body_offset: Option<u64>,
        header_bytes_remaining: u64,
        captured_len: u64,
    ) {
        let max_operation_bytes = self.max_operation_bytes;
        let Some(continuation) = self.continuations.get_mut(stream_key) else {
            return;
        };
        if continuation.message_key != *message_key {
            return;
        }
        let capture_body_limit = continuation.capture_body_limit(max_operation_bytes);
        let captured_body_bytes = captured_len.saturating_sub(header_bytes_remaining);
        let body_end = match body_offset {
            Some(body_offset) => body_offset.saturating_add(captured_body_bytes),
            None => continuation
                .confirmed_body_end
                .saturating_add(captured_body_bytes),
        }
        .min(capture_body_limit);
        continuation.confirmed_body_end = continuation.confirmed_body_end.max(body_end);
        if continuation.confirmed_body_end >= capture_body_limit {
            self.continuations.remove(stream_key);
        }
    }

    fn remove_matching_continuation(
        &mut self,
        stream_key: &SocketContinuationKey,
        message_key: &SocketCaptureKey,
    ) {
        if self
            .continuations
            .get(stream_key)
            .is_some_and(|continuation| continuation.message_key == *message_key)
        {
            self.continuations.remove(stream_key);
        }
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
        let pending = self
            .captures
            .values()
            .fold(0usize, |count, queue| count.saturating_add(queue.len()));
        if pending > limit {
            return Err(ControlError::new(
                "seccomp_socket_pending",
                format!(
                    "pending socket operations {} exceed configured limit {limit}",
                    pending
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
        message_key: SocketCaptureKey,
        content_length: u64,
        header_len: u64,
        linear_payload_range: Option<(u64, u64)>,
    },
    Continuation {
        stream_key: SocketContinuationKey,
        message_key: SocketCaptureKey,
        read_limit: u64,
        body_offset: Option<u64>,
        header_bytes_remaining: u64,
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
    message_key: SocketCaptureKey,
    content_length: u64,
    header_len: u64,
    linear_payload_range: Option<(u64, u64)>,
    confirmed_body_end: u64,
}

impl HttpBodyContinuation {
    fn capture_body_limit(&self, max_operation_bytes: u32) -> u64 {
        self.content_length.min(u64::from(max_operation_bytes))
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SocketCaptureKey {
    pid: u32,
    tid: u32,
    fd: u32,
    syscall: u32,
    buffer_ptr: u64,
    requested_size: u64,
}

impl SocketCaptureKey {
    fn from_request(pid: u32, tid: u32, request: &SocketReadRequest) -> Self {
        Self {
            pid,
            tid,
            fd: request.fd,
            syscall: request.syscall,
            buffer_ptr: request.key_buffer_ptr,
            requested_size: request.key_requested_size,
        }
    }

    fn from_completion(completion: &SocketPayloadCompletion) -> Self {
        Self {
            pid: completion.pid,
            tid: completion.tid,
            fd: completion.fd,
            syscall: completion.syscall,
            buffer_ptr: completion.buffer_ptr,
            requested_size: completion.requested_size,
        }
    }
}

fn linear_suffix_offset(
    initial_range: Option<(u64, u64)>,
    current_range: Option<(u64, u64)>,
) -> Option<u64> {
    let (initial_ptr, initial_size) = initial_range?;
    let (current_ptr, current_size) = current_range?;
    let offset = current_ptr.checked_sub(initial_ptr)?;
    if offset == 0 || offset >= initial_size {
        return None;
    }
    let current_end = offset.checked_add(current_size)?;
    (current_end <= initial_size).then_some(offset)
}

#[cfg(test)]
mod tests {
    use ebpf_collector::SOCKET_PAYLOAD_SYSCALL_SENDTO;

    use super::*;

    #[test]
    fn captures_linear_suffix_before_initial_completion_is_applied() {
        let mut service = test_service();
        let stream_key = test_stream_key();
        let body_len = 120_499;
        let header = request_header(body_len, 961);
        let request_size = header.len() as u64 + body_len;
        let initial_ptr = 0x25_724f_40f00;
        let initial_key = test_capture_key(initial_ptr, request_size);
        let initial_update = service.http_message_capture_update(
            &stream_key,
            stream_key.fd_generation,
            &initial_key,
            Some((initial_ptr, request_size)),
            &header,
        );

        // The first notification installs provisional state before the syscall is resumed.
        service.prepare_capture_update(&initial_update);

        // This is the ordering from the reported trace: the retry notification is drained
        // before the first syscall completion is polled from the BPF ring buffer.
        let first_completed = 89_060;
        let suffix_size = request_size - first_completed;
        let suffix_update = service
            .continuation_capture_update(
                &stream_key,
                Some((initial_ptr + first_completed, suffix_size)),
            )
            .expect("the contiguous suffix must be captured while completion is pending");
        match &suffix_update {
            SocketCaptureUpdate::Continuation {
                read_limit,
                body_offset,
                header_bytes_remaining,
                ..
            } => {
                assert_eq!(*read_limit, suffix_size);
                assert_eq!(*body_offset, Some(first_completed - header.len() as u64));
                assert_eq!(*header_bytes_remaining, 0);
            }
            other => panic!("expected continuation update, got {other:?}"),
        }

        service
            .apply_capture_update(
                initial_update,
                first_completed,
                first_completed,
                first_completed,
            )
            .unwrap();
        let continuation = service
            .continuations
            .get(&stream_key)
            .expect("the suffix is still outstanding");
        assert_eq!(
            continuation.confirmed_body_end,
            first_completed - header.len() as u64
        );

        service
            .apply_capture_update(suffix_update, suffix_size, suffix_size, suffix_size)
            .unwrap();
        assert!(!service.continuations.contains_key(&stream_key));
    }

    #[test]
    fn captures_relocated_suffix_while_initial_completion_is_pending() {
        let mut service = test_service();
        let stream_key = test_stream_key();
        let body_len = 135_200;
        let header = request_header(body_len, 961);
        let request_size = header.len() as u64 + body_len;
        let initial_ptr = 0x25_724f_40f00;
        let initial_key = test_capture_key(initial_ptr, request_size);
        let initial_update = service.http_message_capture_update(
            &stream_key,
            stream_key.fd_generation,
            &initial_key,
            Some((initial_ptr, request_size)),
            &header,
        );
        service.prepare_capture_update(&initial_update);

        let first_completed = 89_060;
        let suffix_size = request_size - first_completed;
        let suffix_update = service
            .continuation_capture_update(&stream_key, Some((0x99_0000, suffix_size)))
            .expect("same-stream body continuation must not depend on buffer identity");
        match &suffix_update {
            SocketCaptureUpdate::Continuation {
                read_limit,
                body_offset,
                header_bytes_remaining,
                ..
            } => {
                assert_eq!(*read_limit, body_len);
                assert_eq!(*body_offset, None);
                assert_eq!(*header_bytes_remaining, 0);
            }
            other => panic!("expected continuation update, got {other:?}"),
        }

        service
            .apply_capture_update(
                initial_update,
                first_completed,
                first_completed,
                first_completed,
            )
            .unwrap();
        service
            .apply_capture_update(suffix_update, suffix_size, suffix_size, suffix_size)
            .unwrap();
        assert!(!service.continuations.contains_key(&stream_key));
    }

    #[test]
    fn same_key_captures_complete_in_notification_order() {
        let mut service = test_service();
        let key = test_capture_key(0x25_724f_40f00, 2);
        service.queue_capture(key.clone(), test_captured_operation(vec![1]));
        service.queue_capture(key.clone(), test_captured_operation(vec![2]));

        assert_eq!(service.take_capture(&key).unwrap().bytes, vec![1]);
        assert!(service.captures.contains_key(&key));
        assert_eq!(service.take_capture(&key).unwrap().bytes, vec![2]);
        assert!(!service.captures.contains_key(&key));
    }

    fn test_service() -> SeccompSocketService {
        SeccompSocketService {
            enabled: true,
            max_operation_bytes: 4 * 1024 * 1024,
            max_segment_bytes: 4095,
            http_sniff_max_bytes: 65_536,
            max_pending_operations: 1024,
            max_stream_states: 1024,
            captures: BTreeMap::new(),
            continuations: BTreeMap::new(),
        }
    }

    fn test_stream_key() -> SocketContinuationKey {
        SocketContinuationKey {
            trace_id: 1,
            pid: 2_715_018,
            fd: 19,
            fd_generation: 11,
            direction: SOCKET_PAYLOAD_DIRECTION_OUTBOUND,
        }
    }

    fn test_capture_key(buffer_ptr: u64, requested_size: u64) -> SocketCaptureKey {
        SocketCaptureKey {
            pid: 2_715_018,
            tid: 2_715_018,
            fd: 19,
            syscall: SOCKET_PAYLOAD_SYSCALL_SENDTO,
            buffer_ptr,
            requested_size,
        }
    }

    fn test_captured_operation(bytes: Vec<u8>) -> CapturedSocketOperation {
        CapturedSocketOperation {
            trace_id: TraceId::new(1),
            process: ProcessObservation::default(),
            bytes,
            protocol_hint: None,
            update: SocketCaptureUpdate::None,
        }
    }

    fn request_header(content_length: u64, header_len: usize) -> Vec<u8> {
        let mut header =
            format!("POST /v1/messages HTTP/1.1\r\nContent-Length: {content_length}\r\nX-Pad: ")
                .into_bytes();
        assert!(header.len() + 4 <= header_len);
        header.resize(header_len - 4, b'x');
        header.extend_from_slice(b"\r\n\r\n");
        assert_eq!(header.len(), header_len);
        header
    }
}

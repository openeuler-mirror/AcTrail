//! Seccomp user-notify TLS payload capture.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{DiagnosticLogLevel, PayloadTlsConfig};
use control_contract::reply::ControlError;
use ebpf_collector::{
    EbpfCollector, TlsPayloadCaptureRequest, TlsPayloadCompletion, TlsPayloadDirectCapture,
};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use process_identity_contract::lookup::{IdentityLookupError, ProcessIdentityReader};

use crate::services::diagnostic_logging;
use crate::services::seccomp_notify::{read_iovec_payload, read_linear_payload};

const TLS_SYMBOL_RUSTLS_WRITE_VECTORED: u32 = 6;
const TLS_CAPTURE_STATE_BPF_COPIED_FULL: u32 = 2;

#[derive(Debug)]
pub(crate) struct SeccompTlsService {
    enabled: bool,
    diagnostics_enabled: bool,
    diagnostic_log_level: DiagnosticLogLevel,
    max_operation_bytes: u32,
    max_segment_bytes: u32,
    max_pending_operations: u32,
    captures: BTreeMap<u64, CapturedTlsOperation>,
    completions: BTreeMap<u64, TlsPayloadCompletion>,
}

impl SeccompTlsService {
    pub(crate) fn new(config: &PayloadTlsConfig, diagnostic_log_level: DiagnosticLogLevel) -> Self {
        Self {
            enabled: config.enabled && !config.capture_backend.is_sync(),
            diagnostics_enabled: config.diagnostics_enabled,
            diagnostic_log_level,
            max_operation_bytes: config.max_operation_bytes,
            max_segment_bytes: config.max_segment_bytes,
            max_pending_operations: config.pending_operation_max_entries,
            captures: BTreeMap::new(),
            completions: BTreeMap::new(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn handle_notification(
        &mut self,
        collector: &EbpfCollector,
        notification: &libc::seccomp_notif,
    ) -> Result<bool, ControlError> {
        if !self.enabled {
            return Ok(false);
        }
        self.capture_if_tls_pending(collector, notification)
    }

    pub(crate) fn ingest_completions(
        &mut self,
        completions: Vec<TlsPayloadCompletion>,
    ) -> Result<(), ControlError> {
        for completion in completions {
            if completion.failed {
                self.log_diagnostic(format_args!(
                    "tls_payload_join drop_failed operation_id={}",
                    completion.operation_id
                ));
                self.captures.remove(&completion.operation_id);
                continue;
            }
            self.log_diagnostic(format_args!(
                "tls_payload_join completion operation_id={} pid={} generation={} completed_size={}",
                completion.operation_id,
                completion.pid,
                completion.pid_generation,
                completion.completed_size
            ));
            self.completions.insert(completion.operation_id, completion);
        }
        self.ensure_pending_capacity()
    }

    pub(crate) fn ingest_capture_requests(
        &mut self,
        requests: Vec<TlsPayloadCaptureRequest>,
    ) -> Result<(), ControlError> {
        for request in requests {
            let read_result = self.capture_stopped_request(&request);
            let continue_result = continue_stopped_process(request.pid);
            continue_result?;
            read_result?;
        }
        self.ensure_pending_capacity()
    }

    pub(crate) fn ingest_direct_captures(
        &mut self,
        captures: Vec<TlsPayloadDirectCapture>,
    ) -> Result<(), ControlError> {
        for capture in captures {
            self.log_diagnostic(format_args!(
                "tls_payload_join direct_capture operation_id={} pid={} generation={} bytes={}",
                capture.operation_id,
                capture.pid,
                capture.pid_generation,
                capture.bytes.len()
            ));
            self.captures.insert(
                capture.operation_id,
                CapturedTlsOperation {
                    bytes: capture.bytes,
                },
            );
        }
        self.ensure_pending_capacity()
    }

    pub(crate) fn complete_operations(
        &mut self,
        identity_reader: &impl ProcessIdentityReader,
    ) -> Result<Vec<RawPayloadSegment>, ControlError> {
        let mut segments = Vec::new();
        let mut ready_operations = self
            .captures
            .keys()
            .filter_map(|operation_id| {
                self.completions.get(operation_id).map(|completion| {
                    (
                        *operation_id,
                        completion.observed_ktime_ns,
                        completion.operation_id,
                    )
                })
            })
            .collect::<Vec<_>>();
        ready_operations.sort_by_key(|(_, observed_ktime_ns, operation_id)| {
            (*observed_ktime_ns, *operation_id)
        });
        if !ready_operations.is_empty() {
            self.log_diagnostic(format_args!(
                "tls_payload_join ready_operations={}",
                ready_operations.len()
            ));
        }
        for (operation_id, _, _) in ready_operations {
            let completion = self.completions.remove(&operation_id).ok_or_else(|| {
                ControlError::new("seccomp_tls_segment", "completion disappeared")
            })?;
            let Some(capture) = self.captures.remove(&completion.operation_id) else {
                continue;
            };
            let operation_original_size = completion.completed_size;
            let operation_captured_size = (capture.bytes.len() as u64).min(operation_original_size);
            let captured_len = usize::try_from(operation_captured_size).map_err(|error| {
                ControlError::new(
                    "seccomp_tls_segment",
                    format!("captured size overflow: {error}"),
                )
            })?;
            let captured_bytes = &capture.bytes[..captured_len];
            let completion_state = if operation_captured_size == operation_original_size {
                PayloadOperationCompletionState::Success
            } else {
                PayloadOperationCompletionState::Partial
            };
            let Some(process) = tls_completion_identity(&completion, identity_reader)? else {
                continue;
            };
            let segment_max = usize::try_from(self.max_segment_bytes).map_err(|error| {
                ControlError::new(
                    "seccomp_tls_segment",
                    format!("segment size overflow: {error}"),
                )
            })?;
            if segment_max == 0 {
                return Err(ControlError::new(
                    "seccomp_tls_segment",
                    "payload_tls_max_segment_bytes must be positive",
                ));
            }
            for (index, chunk) in captured_bytes.chunks(segment_max).enumerate() {
                let offset = index
                    .checked_mul(segment_max)
                    .ok_or_else(|| ControlError::new("seccomp_tls_segment", "offset overflow"))?;
                let final_chunk = offset + chunk.len() >= captured_bytes.len();
                let truncation = if final_chunk && operation_captured_size < operation_original_size
                {
                    PayloadTruncationState::Truncated
                } else {
                    PayloadTruncationState::Complete
                };
                segments.push(RawPayloadSegment {
                    trace_id: completion.trace_id,
                    observed_at: SystemTime::now(),
                    process: process.clone(),
                    source_boundary: PayloadSourceBoundary::TlsUserSpace,
                    content_state: PayloadContentState::Plaintext,
                    direction: tls_direction(completion.direction)?,
                    stream_key: PayloadStreamKey::new(format!(
                        "tls:{}:{:x}",
                        completion.pid, completion.stream_key
                    )),
                    sequence: completion.operation_id + index as u64,
                    original_size: if truncation == PayloadTruncationState::Truncated {
                        operation_original_size.saturating_sub(offset as u64)
                    } else {
                        chunk.len() as u64
                    },
                    captured_size: chunk.len() as u64,
                    operation_id: completion.operation_id,
                    operation_offset: offset as u64,
                    operation_original_size,
                    operation_captured_size,
                    operation_completion_state: completion_state,
                    truncation,
                    library: tls_library(completion.library)?.to_string(),
                    symbol: tls_symbol(completion.symbol)?.to_string(),
                    protocol_hint: None,
                    bytes: chunk.to_vec(),
                });
            }
            self.log_diagnostic(format_args!(
                "tls_payload_join emitted operation_id={} segments={} captured_size={} original_size={}",
                completion.operation_id,
                segments.len(),
                operation_captured_size,
                operation_original_size
            ));
        }
        Ok(segments)
    }

    fn capture_if_tls_pending(
        &mut self,
        collector: &EbpfCollector,
        notification: &libc::seccomp_notif,
    ) -> Result<bool, ControlError> {
        let Some(pending) = collector
            .lookup_pending_tls_payload_op(notification.pid)
            .map_err(|error| ControlError::new(error.stage, error.message))?
        else {
            return Ok(false);
        };
        if pending.capture_state == TLS_CAPTURE_STATE_BPF_COPIED_FULL {
            return Ok(true);
        }
        let Some(bytes) = self.capture_request_bytes(
            pending.tgid,
            pending.symbol,
            pending.buffer_ptr,
            pending.requested_size,
        )?
        else {
            return Ok(false);
        };
        self.captures
            .insert(pending.operation_id, CapturedTlsOperation { bytes });
        Ok(true)
    }

    fn ensure_pending_capacity(&self) -> Result<(), ControlError> {
        let pending = self.completions.len().max(self.captures.len());
        let limit = usize::try_from(self.max_pending_operations).map_err(|error| {
            ControlError::new(
                "seccomp_tls_pending",
                format!("pending operation limit overflow: {error}"),
            )
        })?;
        if pending > limit {
            return Err(ControlError::new(
                "seccomp_tls_pending",
                format!("pending TLS operations {pending} exceed configured limit {limit}"),
            ));
        }
        Ok(())
    }

    fn capture_stopped_request(
        &mut self,
        request: &TlsPayloadCaptureRequest,
    ) -> Result<(), ControlError> {
        let Some(bytes) = self.capture_request_bytes(
            request.pid,
            request.symbol,
            request.buffer_ptr,
            request.requested_size,
        )?
        else {
            return Ok(());
        };
        self.captures
            .insert(request.operation_id, CapturedTlsOperation { bytes });
        Ok(())
    }

    fn capture_request_bytes(
        &self,
        pid: u32,
        symbol: u32,
        buffer_ptr: u64,
        requested_size: u64,
    ) -> Result<Option<Vec<u8>>, ControlError> {
        if symbol == TLS_SYMBOL_RUSTLS_WRITE_VECTORED {
            let iovec_count = usize::try_from(requested_size).map_err(|error| {
                ControlError::new("seccomp_tls_read", format!("iovec count overflow: {error}"))
            })?;
            return read_iovec_payload(pid, buffer_ptr, iovec_count, self.max_operation_bytes);
        }
        read_linear_payload(pid, buffer_ptr, requested_size, self.max_operation_bytes)
    }

    fn log_diagnostic(&self, args: std::fmt::Arguments<'_>) {
        if self.diagnostics_enabled {
            diagnostic_logging::log_diagnostic(
                self.diagnostic_log_level,
                DiagnosticLogLevel::Debug,
                args,
            );
        }
    }
}

#[derive(Debug)]
struct CapturedTlsOperation {
    bytes: Vec<u8>,
}

fn continue_stopped_process(pid: u32) -> Result<(), ControlError> {
    let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGCONT) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    // The target exited while stopped for capture: there is nothing left to resume, so a missing
    // process (ESRCH) is benign rather than a fatal error that would crash the daemon.
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(ControlError::new("seccomp_tls_continue", error.to_string()))
}

/// Returns `Ok(None)` when the target exited before its identity could be read: the segment is
/// dropped rather than crashing the daemon. Only reachable when `pid_generation` is unset (0).
fn tls_completion_identity(
    completion: &TlsPayloadCompletion,
    identity_reader: &impl ProcessIdentityReader,
) -> Result<Option<ProcessIdentity>, ControlError> {
    if completion.pid_generation != 0 {
        return Ok(Some(ProcessIdentity::new(
            completion.pid,
            completion.pid_generation,
            completion.pid_generation,
        )));
    }
    match identity_reader.read_identity(completion.pid) {
        Ok(identity) => Ok(Some(identity)),
        Err(IdentityLookupError::NotFound { .. }) => Ok(None),
        Err(error) => Err(ControlError::new(
            "seccomp_tls_identity",
            format!("{error:?}"),
        )),
    }
}

fn tls_direction(raw: u32) -> Result<PayloadDirection, ControlError> {
    match raw {
        1 => Ok(PayloadDirection::Outbound),
        2 => Ok(PayloadDirection::Inbound),
        other => Err(ControlError::new(
            "seccomp_tls_direction",
            format!("unknown TLS direction {other}"),
        )),
    }
}

fn tls_symbol(raw: u32) -> Result<&'static str, ControlError> {
    match raw {
        1 => Ok("SSL_write"),
        2 => Ok("SSL_read"),
        3 => Ok("SSL_write_ex"),
        4 => Ok("SSL_read_ex"),
        5 => Ok("rustls_plaintext_write"),
        6 => Ok("rustls_plaintext_write_vectored"),
        other => Err(ControlError::new(
            "seccomp_tls_symbol",
            format!("unknown TLS symbol {other}"),
        )),
    }
}

fn tls_library(raw: u32) -> Result<&'static str, ControlError> {
    match raw {
        1 => Ok("openssl"),
        2 => Ok("boringssl"),
        3 => Ok("rustls"),
        other => Err(ControlError::new(
            "seccomp_tls_library",
            format!("unknown TLS library {other}"),
        )),
    }
}

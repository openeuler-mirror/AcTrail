//! Sync TLS payload event ingestion.

use std::fs::{self, Permissions};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::SystemTime;

use config_core::daemon::{PayloadTlsCaptureBackend, PayloadTlsConfig};
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use tls_payload_core::PayloadDirection as SyncDirection;
use tls_payload_sync::{PayloadEvent, SyncEvent, decode_event_line};

pub(crate) struct TlsSyncService {
    listener: Option<UnixListener>,
    clients: Vec<TlsSyncClient>,
    read_buffer_bytes: usize,
    max_line_bytes: usize,
}

impl TlsSyncService {
    pub(crate) fn new(config: &PayloadTlsConfig) -> Result<Self, ControlError> {
        if !enabled(config) {
            return Ok(Self {
                listener: None,
                clients: Vec::new(),
                read_buffer_bytes: usize::default(),
                max_line_bytes: usize::default(),
            });
        }
        let listener = UnixListener::bind(&config.sync_event_socket_path)
            .map_err(|error| ControlError::new("tls_sync_bind", error.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| ControlError::new("tls_sync_nonblocking", error.to_string()))?;
        fs::set_permissions(
            &config.sync_event_socket_path,
            Permissions::from_mode(config.sync_socket_mode),
        )
        .map_err(|error| ControlError::new("tls_sync_permissions", error.to_string()))?;
        Ok(Self {
            listener: Some(listener),
            clients: Vec::new(),
            read_buffer_bytes: read_buffer_bytes(config)?,
            max_line_bytes: max_line_bytes(config)?,
        })
    }

    pub(crate) fn event_poll_fd(&self) -> Option<RawFd> {
        self.listener.as_ref().map(AsRawFd::as_raw_fd)
    }

    pub(crate) fn drain(&mut self) -> Result<Vec<RawPayloadSegment>, ControlError> {
        self.accept_ready_clients()?;
        let mut segments = Vec::new();
        let mut retained_clients = Vec::new();
        for mut client in std::mem::take(&mut self.clients) {
            let closed =
                client.read_events(self.read_buffer_bytes, self.max_line_bytes, &mut segments)?;
            if !closed {
                retained_clients.push(client);
            }
        }
        self.clients = retained_clients;
        Ok(segments)
    }

    fn accept_ready_clients(&mut self) -> Result<(), ControlError> {
        let Some(listener) = &self.listener else {
            return Ok(());
        };
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream
                        .set_nonblocking(true)
                        .map_err(|error| ControlError::new("tls_sync_client", error.to_string()))?;
                    self.clients.push(TlsSyncClient {
                        stream,
                        buffer: Vec::new(),
                    });
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(error) => {
                    return Err(ControlError::new("tls_sync_accept", error.to_string()));
                }
            }
        }
    }
}

struct TlsSyncClient {
    stream: UnixStream,
    buffer: Vec<u8>,
}

impl TlsSyncClient {
    fn read_events(
        &mut self,
        read_buffer_bytes: usize,
        max_line_bytes: usize,
        segments: &mut Vec<RawPayloadSegment>,
    ) -> Result<bool, ControlError> {
        let mut scratch = vec![0_u8; read_buffer_bytes];
        loop {
            match self.stream.read(&mut scratch) {
                Ok(0) => return Ok(true),
                Ok(read) => {
                    self.buffer.extend_from_slice(&scratch[..read]);
                    if self.buffer.len() > max_line_bytes {
                        return Err(ControlError::new(
                            "tls_sync_event",
                            "sync event line exceeded configured maximum",
                        ));
                    }
                    self.drain_complete_lines(segments)?;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(ControlError::new("tls_sync_read", error.to_string())),
            }
        }
    }

    fn drain_complete_lines(
        &mut self,
        segments: &mut Vec<RawPayloadSegment>,
    ) -> Result<(), ControlError> {
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=index).collect::<Vec<_>>();
            match decode_event_line(&line).map_err(sync_event_error)? {
                SyncEvent::Payload(event) => segments.push(payload_segment(event)?),
                SyncEvent::Decision(_) => {}
            }
        }
        Ok(())
    }
}

fn payload_segment(event: PayloadEvent) -> Result<RawPayloadSegment, ControlError> {
    let captured_size = event.bytes.len() as u64;
    Ok(RawPayloadSegment {
        trace_id: TraceId::new(event.trace_id),
        observed_at: SystemTime::now(),
        process: ProcessIdentity::new(event.pid, 0, 0),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: payload_direction(event.direction),
        stream_key: PayloadStreamKey::new(format!("tls-sync:{}:{:x}", event.pid, event.stream_key)),
        sequence: event.sequence,
        original_size: captured_size,
        captured_size,
        operation_id: event.sequence,
        operation_offset: 0,
        operation_original_size: captured_size,
        operation_captured_size: captured_size,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        library: event.provider,
        symbol: event.symbol,
        protocol_hint: None,
        bytes: event.bytes,
    })
}

fn payload_direction(direction: SyncDirection) -> PayloadDirection {
    match direction {
        SyncDirection::Outbound => PayloadDirection::Outbound,
        SyncDirection::Inbound => PayloadDirection::Inbound,
    }
}

fn enabled(config: &PayloadTlsConfig) -> bool {
    config.enabled && config.capture_backend == PayloadTlsCaptureBackend::TlsSync
}

fn read_buffer_bytes(config: &PayloadTlsConfig) -> Result<usize, ControlError> {
    usize::try_from(config.max_segment_bytes)
        .map_err(|error| ControlError::new("tls_sync_config", error.to_string()))
}

fn max_line_bytes(config: &PayloadTlsConfig) -> Result<usize, ControlError> {
    let operation = usize::try_from(config.max_operation_bytes)
        .map_err(|error| ControlError::new("tls_sync_config", error.to_string()))?;
    let segment = usize::try_from(config.max_segment_bytes)
        .map_err(|error| ControlError::new("tls_sync_config", error.to_string()))?;
    operation
        .checked_mul(2)
        .and_then(|value| value.checked_add(segment))
        .ok_or_else(|| ControlError::new("tls_sync_config", "sync event line limit overflow"))
}

fn sync_event_error(error: tls_payload_sync::SyncError) -> ControlError {
    ControlError::new("tls_sync_event", error.to_string())
}

//! Sync TLS payload event ingestion.

#[path = "plan_store.rs"]
mod plan_store;
#[path = "resolver.rs"]
mod resolver;

use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::SystemTime;

use config_core::daemon::{PayloadTlsCaptureBackend, PayloadTlsConfig};
use control_contract::reply::ControlError;
use ebpf_collector::procfs::resolve_namespaced_pid;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use tls_payload_core::PayloadDirection as SyncDirection;
use tls_payload_sync::{
    PayloadEvent, SummaryEvent, SyncEvent, decode_event_line, decode_plan_lookup_request,
};
use trace_runtime::registry::TraceRuntime;
use uds_control_server::PeerCredentials;

use crate::peer_identity::{PeerIdentity, peer_error};
use self::resolver::TlsSyncPlanResolver;

pub(crate) struct TlsSyncService {
    listener: Option<UnixListener>,
    clients: Vec<TlsSyncClient>,
    resolver: Option<TlsSyncPlanResolver>,
    read_buffer_bytes: usize,
    max_line_bytes: usize,
}

#[derive(Debug, Default)]
pub(crate) struct TlsSyncDrain {
    pub(crate) payload_segments: Vec<RawPayloadSegment>,
    pub(crate) diagnostics: Vec<TlsSyncDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TlsSyncDiagnostic {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl TlsSyncService {
    pub(crate) fn new(config: &PayloadTlsConfig) -> Result<Self, ControlError> {
        if !enabled(config) {
            return Ok(Self {
                listener: None,
                clients: Vec::new(),
                resolver: None,
                read_buffer_bytes: usize::default(),
                max_line_bytes: usize::default(),
            });
        }
        let resolver = TlsSyncPlanResolver::new(config)?;
        create_parent_directory(&config.sync_event_socket_path)?;
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
            resolver: Some(resolver),
            read_buffer_bytes: read_buffer_bytes(config)?,
            max_line_bytes: max_line_bytes(config)?,
        })
    }

    pub(crate) fn event_poll_fds(&self) -> Vec<RawFd> {
        let mut fds = self
            .listener
            .as_ref()
            .map(AsRawFd::as_raw_fd)
            .into_iter()
            .collect::<Vec<_>>();
        fds.extend(self.clients.iter().map(TlsSyncClient::event_poll_fd));
        fds
    }

    pub(crate) fn prewarm_plan_for_exec(
        &self,
        binary: &std::path::Path,
    ) -> Result<(), ControlError> {
        let Some(resolver) = &self.resolver else {
            return Ok(());
        };
        resolver.prewarm(binary)
    }

    pub(crate) fn drain(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<TlsSyncDrain, ControlError> {
        self.accept_ready_clients()?;
        let mut drain = TlsSyncDrain::default();
        let mut retained_clients = Vec::new();
        for mut client in std::mem::take(&mut self.clients) {
            let result = client.read_events(
                self.read_buffer_bytes,
                self.max_line_bytes,
                self.resolver.as_ref(),
                trace_runtime,
                &mut drain.payload_segments,
            );
            match result {
                Ok(closed) => {
                    if !closed {
                        retained_clients.push(client);
                    }
                }
                Err(error) => {
                    audit_tls_peer_rejection(&client.peer, &error);
                    drain.diagnostics.push(TlsSyncDiagnostic {
                        code: error.code,
                        message: error.message,
                    });
                }
            }
        }
        self.clients = retained_clients;
        Ok(drain)
    }

    fn accept_ready_clients(&mut self) -> Result<(), ControlError> {
        let Some(listener) = &self.listener else {
            return Ok(());
        };
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let credentials = match PeerCredentials::from_stream(&stream) {
                        Ok(credentials) => credentials,
                        Err(error) => {
                            tracing::warn!(
                                target: "actrail::peer_auth",
                                error = %error,
                                "rejected TLS-sync connection without peer credentials"
                            );
                            continue;
                        }
                    };
                    let peer = match PeerIdentity::resolve(credentials) {
                        Ok(peer) => peer,
                        Err(error) => {
                            audit_tls_credentials_rejection(credentials, &error);
                            continue;
                        }
                    };
                    stream
                        .set_nonblocking(true)
                        .map_err(|error| ControlError::new("tls_sync_client", error.to_string()))?;
                    self.clients.push(TlsSyncClient {
                        stream,
                        peer,
                        buffer: Vec::new(),
                        process_cache: BTreeMap::new(),
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
    peer: PeerIdentity,
    buffer: Vec<u8>,
    process_cache: BTreeMap<TlsSyncProcessCacheKey, ProcessIdentity>,
}

impl TlsSyncClient {
    fn event_poll_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    fn read_events(
        &mut self,
        read_buffer_bytes: usize,
        max_line_bytes: usize,
        resolver: Option<&TlsSyncPlanResolver>,
        trace_runtime: &TraceRuntime,
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
                    if self.drain_complete_lines(resolver, trace_runtime, segments)? {
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(ControlError::new("tls_sync_read", error.to_string())),
            }
        }
    }

    fn drain_complete_lines(
        &mut self,
        resolver: Option<&TlsSyncPlanResolver>,
        trace_runtime: &TraceRuntime,
        segments: &mut Vec<RawPayloadSegment>,
    ) -> Result<bool, ControlError> {
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=index).collect::<Vec<_>>();
            if let Ok(request) = decode_plan_lookup_request(&line) {
                let Some(resolver) = resolver else {
                    return Err(ControlError::new(
                        "tls_sync_plan",
                        "plan lookup received while resolver is disabled",
                    ));
                };
                let response = self
                    .stream
                    .try_clone()
                    .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
                response
                    .set_nonblocking(false)
                    .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
                resolver.submit_lookup(&request.binary, response)?;
                return Ok(true);
            }
            let event = decode_event_line(&line).map_err(sync_event_error)?;
            authorize_sync_event(&self.peer, trace_runtime, &event)?;
            match event {
                SyncEvent::Payload(event) => {
                    if let Some(segment) =
                        payload_segment(event, trace_runtime, &mut self.process_cache)?
                    {
                        segments.push(segment);
                    }
                }
                SyncEvent::Summary(event) => {
                    if let Some(segment) =
                        summary_segment(event, trace_runtime, &mut self.process_cache)?
                    {
                        segments.push(segment);
                    }
                }
                SyncEvent::Decision(_) => {}
            }
        }
        Ok(false)
    }
}

fn authorize_sync_event(
    peer: &PeerIdentity,
    trace_runtime: &TraceRuntime,
    event: &SyncEvent,
) -> Result<(), ControlError> {
    let trace_id = match event {
        SyncEvent::Payload(event) => TraceId::new(event.trace_id),
        SyncEvent::Summary(event) => TraceId::new(event.trace_id),
        SyncEvent::Decision(event) => TraceId::new(event.trace_id),
    };
    let trace = trace_runtime
        .get_trace(trace_id)
        .ok_or_else(|| peer_error(format!("TLS-sync event references unknown trace {trace_id}")))?;
    let owner = trace
        .owner
        .as_ref()
        .ok_or_else(|| peer_error(format!("trace {trace_id} has no live peer binding")))?;
    peer.authorize_trace_owner(trace_id, owner)
}

fn audit_tls_peer_rejection(peer: &PeerIdentity, error: &ControlError) {
    tracing::warn!(
        target: "actrail::peer_auth",
        peer_pid = peer.credentials.pid,
        peer_uid = peer.credentials.uid,
        peer_gid = peer.credentials.gid,
        peer_container = peer.principal.container_id.as_deref().unwrap_or("host"),
        error_code = %error.code,
        error = %error.message,
        "closed rejected TLS-sync peer"
    );
}

fn audit_tls_credentials_rejection(credentials: PeerCredentials, error: &ControlError) {
    tracing::warn!(
        target: "actrail::peer_auth",
        peer_pid = credentials.pid,
        peer_uid = credentials.uid,
        peer_gid = credentials.gid,
        error_code = %error.code,
        error = %error.message,
        "rejected TLS-sync peer identity"
    );
}

fn summary_segment(
    event: SummaryEvent,
    trace_runtime: &TraceRuntime,
    process_cache: &mut BTreeMap<TlsSyncProcessCacheKey, ProcessIdentity>,
) -> Result<Option<RawPayloadSegment>, ControlError> {
    let trace_id = TraceId::new(event.trace_id);
    let process = match resolve_tls_sync_process(trace_runtime, trace_id, event.pid, process_cache)
    {
        Ok(process) => process,
        Err(error) if stale_namespaced_pid_resolution(&error) => {
            tracing::warn!(
                target: "actrail::tls_sync",
                trace_id = trace_id.get(),
                namespace_pid = event.pid,
                error = %error.message,
                "dropped stale TLS sync summary event after process exit"
            );
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let captured_size = event.bytes.len() as u64;
    let original_size = event.observed_size.max(captured_size);
    Ok(Some(RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::now(),
        process: process.clone(),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: payload_direction(event.direction),
        stream_key: PayloadStreamKey::new(format!(
            "tls-sync:{}:{:x}",
            process.pid, event.stream_key
        )),
        sequence: event.sequence,
        original_size,
        captured_size,
        operation_id: event.sequence,
        operation_offset: 0,
        operation_original_size: original_size,
        operation_captured_size: captured_size,
        operation_completion_state: PayloadOperationCompletionState::Partial,
        truncation: PayloadTruncationState::Truncated,
        library: event.provider,
        symbol: event.symbol,
        protocol_hint: Some(format!(
            "tls-summary;reason={};protocol={}",
            event.reason, event.protocol_hint
        )),
        bytes: event.bytes,
    }))
}

fn payload_segment(
    event: PayloadEvent,
    trace_runtime: &TraceRuntime,
    process_cache: &mut BTreeMap<TlsSyncProcessCacheKey, ProcessIdentity>,
) -> Result<Option<RawPayloadSegment>, ControlError> {
    let trace_id = TraceId::new(event.trace_id);
    let process = match resolve_tls_sync_process(trace_runtime, trace_id, event.pid, process_cache)
    {
        Ok(process) => process,
        Err(error) if stale_namespaced_pid_resolution(&error) => {
            tracing::warn!(
                target: "actrail::tls_sync",
                trace_id = trace_id.get(),
                namespace_pid = event.pid,
                error = %error.message,
                "dropped stale TLS sync payload event after process exit"
            );
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let captured_size = event.bytes.len() as u64;
    Ok(Some(RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::now(),
        process: process.clone(),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: payload_direction(event.direction),
        stream_key: PayloadStreamKey::new(format!(
            "tls-sync:{}:{:x}",
            process.pid, event.stream_key
        )),
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
    }))
}

fn resolve_tls_sync_process(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
    namespace_pid: u32,
    process_cache: &mut BTreeMap<TlsSyncProcessCacheKey, ProcessIdentity>,
) -> Result<ProcessIdentity, ControlError> {
    let entry = trace_runtime
        .get_trace(trace_id)
        .ok_or_else(|| ControlError::new("tls_sync_pid_resolution", "trace not found"))?;
    let cache_key = TlsSyncProcessCacheKey {
        trace_id,
        namespace_pid,
    };
    if let Some(process) = process_cache.get(&cache_key) {
        return Ok(process.clone());
    }
    let pid_namespace = entry
        .trace
        .root_process_identity
        .pid_namespace
        .as_ref()
        .ok_or_else(|| {
            ControlError::new(
                "tls_sync_pid_resolution",
                "trace root process is missing PID namespace metadata",
            )
        })?;
    let process = resolve_namespaced_pid(namespace_pid, pid_namespace)
        .map_err(|error| ControlError::new("tls_sync_pid_resolution", error))?;
    process_cache.insert(cache_key, process.clone());
    Ok(process)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TlsSyncProcessCacheKey {
    trace_id: TraceId,
    namespace_pid: u32,
}

fn stale_namespaced_pid_resolution(error: &ControlError) -> bool {
    error.code == "tls_sync_pid_resolution"
        && error
            .message
            .starts_with("no host process matched namespace pid ")
}

fn payload_direction(direction: SyncDirection) -> PayloadDirection {
    match direction {
        SyncDirection::Outbound => PayloadDirection::Outbound,
        SyncDirection::Inbound => PayloadDirection::Inbound,
    }
}

fn create_parent_directory(socket_path: &std::path::Path) -> Result<(), ControlError> {
    let Some(parent) = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        ControlError::new(
            "tls_sync_directory",
            format!(
                "create TLS sync socket directory {} failed: {error}",
                parent.display()
            ),
        )
    })
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

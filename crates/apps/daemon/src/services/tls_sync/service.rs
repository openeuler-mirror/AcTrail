//! Sync TLS payload event ingestion.

#[path = "direct.rs"]
mod direct;
#[path = "frame_buffer.rs"]
mod frame_buffer;
#[path = "resolver.rs"]
mod resolver;
#[path = "root_path.rs"]
mod root_path;
pub(in crate::services) use direct::{DirectObject, DirectResolution};

use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::SystemTime;

use config_core::daemon::{PayloadTlsCaptureBackend, PayloadTlsConfig};
use control_contract::reply::{ControlError, LaunchTlsPlanReply};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::{NamespaceIdentity, NamespaceProcessCoordinates, ProcessObservation};
use payload_event::RawPayloadSegment;
use storage_core::TlsFlowDiagnostic;
use tls_payload_core::PayloadDirection as SyncDirection;
use tls_payload_sync::{
    FrameCodec, PayloadEvent, SummaryEvent, SyncEvent, SyncMessage, target_runtime_for_path,
};
use trace_runtime::registry::TraceRuntime;
use uds_control_server::PeerCredentials;

use self::frame_buffer::FrameBuffer;
use self::resolver::{ExecPlanConsumer, TlsSyncPlanResolver};
use self::root_path::{PeerRootResolver, PinnedPeerPath};
use crate::peer_identity::{PeerIdentity, peer_error};

pub(crate) struct TlsSyncService {
    listener: Option<UnixListener>,
    clients: Vec<TlsSyncClient>,
    resolver: Option<TlsSyncPlanResolver>,
    read_buffer: Vec<u8>,
    max_frame_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExecTlsPlanMode {
    Direct,
    Sync,
}

pub(crate) struct ExecTlsPlanResolution {
    pub(crate) mode: ExecTlsPlanMode,
    pub(crate) reply: LaunchTlsPlanReply,
}

pub(crate) struct RuntimeRootPathMapper {
    root: root_path::PeerRootHandle,
}

pub(crate) struct PinnedRuntimePath {
    _handle: PinnedPeerPath,
    probe_path: std::path::PathBuf,
}

impl RuntimeRootPathMapper {
    pub(crate) fn pin(&self, runtime_path: &Path) -> Result<PinnedRuntimePath, ControlError> {
        let handle = self
            .root
            .pin_path(runtime_path)
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))?;
        let probe_path = handle.path();
        Ok(PinnedRuntimePath {
            _handle: handle,
            probe_path,
        })
    }
}

impl PinnedRuntimePath {
    pub(crate) fn path(&self) -> &Path {
        &self.probe_path
    }
}

#[derive(Debug, Default)]
pub(crate) struct TlsSyncDrain {
    pub(crate) payload_segments: Vec<RawPayloadSegment>,
    pub(crate) diagnostics: Vec<TlsSyncDiagnostic>,
    pub(crate) flow_diagnostics: Vec<TlsFlowDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TlsSyncDiagnostic {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl TlsSyncService {
    pub(crate) fn new(config: &PayloadTlsConfig) -> Result<Self, ControlError> {
        if !enabled(config) {
            let resolver = if config.enabled && config.capture_backend.uses_probe_plans() {
                Some(TlsSyncPlanResolver::new(config)?)
            } else {
                None
            };
            return Ok(Self {
                listener: None,
                clients: Vec::new(),
                resolver,
                read_buffer: Vec::new(),
                max_frame_bytes: usize::default(),
            });
        }
        let max_frame_bytes = usize::try_from(config.sync_max_frame_bytes)
            .map_err(|error| ControlError::new("tls_sync_config", error.to_string()))?;
        if max_frame_bytes < FrameCodec::HEADER_LEN || config.max_segment_bytes == 0 {
            return Err(ControlError::new(
                "tls_sync_config",
                "sync_max_frame_bytes must fit the frame header and max_segment_bytes must be positive",
            ));
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
            read_buffer: vec![0; read_buffer_bytes(config)?],
            max_frame_bytes,
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
        fds.extend(
            self.resolver
                .as_ref()
                .and_then(TlsSyncPlanResolver::direct_poll_fd),
        );
        fds
    }

    pub(in crate::services) fn submit_direct(
        &self,
        object: DirectObject,
    ) -> Option<DirectResolution> {
        self.resolver.as_ref()?.submit_direct(object)
    }

    pub(in crate::services) fn direct_discovery_enabled(&self) -> bool {
        self.resolver
            .as_ref()
            .is_some_and(TlsSyncPlanResolver::direct_discovery_enabled)
    }

    pub(in crate::services) fn drain_direct(&mut self) -> Vec<DirectResolution> {
        self.resolver
            .as_mut()
            .map(TlsSyncPlanResolver::drain_direct)
            .unwrap_or_default()
    }

    pub(crate) fn resolve_exec_plan(
        &self,
        binary: &Path,
    ) -> Result<ExecTlsPlanResolution, ControlError> {
        let Some(resolver) = &self.resolver else {
            return Err(ControlError::new(
                "tls_sync_plan",
                "TLS sync plan resolver is disabled",
            ));
        };
        let runtime = target_runtime_for_path(binary, None)
            .map_err(|error| ControlError::new("tls_sync_exec_target", error.to_string()))?;
        let (mode, consumer) = if runtime.is_static() {
            (ExecTlsPlanMode::Direct, ExecPlanConsumer::Daemon)
        } else {
            (ExecTlsPlanMode::Sync, ExecPlanConsumer::Sync)
        };
        resolver
            .resolve_exec_plan(&runtime.path, consumer)
            .map(|reply| ExecTlsPlanResolution { mode, reply })
    }

    pub(crate) fn resolve_launch_plan(
        &self,
        binary: &Path,
        path_view_pid: u32,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        let Some(resolver) = &self.resolver else {
            return Err(ControlError::new(
                "tls_sync_plan",
                "TLS sync plan resolver is disabled",
            ));
        };
        let mut root = PeerRootResolver::new(path_view_pid);
        resolver.resolve_launch_plan(binary, root.duplicate())
    }

    pub(crate) fn runtime_root_path_mapper(
        &self,
        path_view_pid: u32,
    ) -> Result<RuntimeRootPathMapper, ControlError> {
        let mut root = PeerRootResolver::new(path_view_pid);
        root.duplicate()
            .map(|root| RuntimeRootPathMapper { root })
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))
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
                &mut self.read_buffer,
                self.max_frame_bytes,
                self.resolver.as_ref(),
                trace_runtime,
                &mut drain.payload_segments,
                &mut drain.flow_diagnostics,
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
                        path_root: PeerRootResolver::new(peer.credentials.pid),
                        peer,
                        buffer: FrameBuffer::default(),
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
    path_root: PeerRootResolver,
    peer: PeerIdentity,
    buffer: FrameBuffer,
    process_cache: BTreeMap<TlsSyncProcessCacheKey, ProcessObservation>,
}

impl TlsSyncClient {
    fn event_poll_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    fn read_events(
        &mut self,
        scratch: &mut [u8],
        max_frame_bytes: usize,
        resolver: Option<&TlsSyncPlanResolver>,
        trace_runtime: &TraceRuntime,
        segments: &mut Vec<RawPayloadSegment>,
        flow_diagnostics: &mut Vec<TlsFlowDiagnostic>,
    ) -> Result<bool, ControlError> {
        loop {
            match self.stream.read(scratch) {
                Ok(0) if self.buffer.is_empty() => return Ok(true),
                Ok(0) => {
                    return Err(ControlError::new(
                        "tls_sync_event",
                        "truncated sync frame at EOF",
                    ));
                }
                Ok(read) => {
                    self.buffer.append(&scratch[..read]);
                    if self.drain_complete_frames(
                        max_frame_bytes,
                        resolver,
                        trace_runtime,
                        segments,
                        flow_diagnostics,
                    )? {
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(ControlError::new("tls_sync_read", error.to_string())),
            }
        }
    }

    fn drain_complete_frames(
        &mut self,
        max_frame_bytes: usize,
        resolver: Option<&TlsSyncPlanResolver>,
        trace_runtime: &TraceRuntime,
        segments: &mut Vec<RawPayloadSegment>,
        flow_diagnostics: &mut Vec<TlsFlowDiagnostic>,
    ) -> Result<bool, ControlError> {
        while let Some(message) = self
            .buffer
            .next(max_frame_bytes)
            .map_err(sync_event_error)?
        {
            let event = match message {
                SyncMessage::Event(event) => event,
                SyncMessage::PlanResponse(_) => {
                    return Err(ControlError::new(
                        "tls_sync_event",
                        "unexpected plan response from runtime",
                    ));
                }
                SyncMessage::PlanLookup(request) => {
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
                    let peer_root = self.path_root.duplicate();
                    resolver.submit_lookup(&request.binary, peer_root, response)?;
                    return Ok(true);
                }
            };
            authorize_sync_event(&self.peer, trace_runtime, &event)?;
            match event {
                SyncEvent::Payload(event) => {
                    if let Some(segment) = payload_segment(event, &mut self.process_cache)? {
                        segments.push(segment);
                    }
                }
                SyncEvent::Summary(event) => {
                    let stream_key = format!("tls-sync:{}:{:x}", event.pid, event.stream_key);
                    flow_diagnostics.push(TlsFlowDiagnostic {
                        trace_id: TraceId::new(event.trace_id),
                        stream_key,
                        direction: match event.direction {
                            SyncDirection::Outbound => 0,
                            SyncDirection::Inbound => 1,
                        },
                        reason_code: tls_flow_reason_code(&event.reason),
                        observed_size: event.observed_size,
                        emitted_size: event.emitted_size,
                        emitted_at: SystemTime::now(),
                    });
                    if let Some(segment) = summary_segment(event, &mut self.process_cache)? {
                        segments.push(segment);
                    }
                }
                SyncEvent::Decision(_) => {}
            }
        }
        self.buffer.compact();
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
    let trace = trace_runtime.get_trace(trace_id).ok_or_else(|| {
        peer_error(format!(
            "TLS-sync event references unknown trace {trace_id}"
        ))
    })?;
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
        peer_pid_namespace = %peer.principal.pid_namespace,
        peer_mount_namespace = %peer.principal.mount_namespace,
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
    process_cache: &mut BTreeMap<TlsSyncProcessCacheKey, ProcessObservation>,
) -> Result<Option<RawPayloadSegment>, ControlError> {
    let trace_id = TraceId::new(event.trace_id);
    let process = resolve_tls_sync_process(
        trace_id,
        event.pid,
        event.start_time_ticks,
        &event.pid_namespace,
        process_cache,
    )?;
    let captured_size = event.bytes.len() as u64;
    let original_size = event.observed_size.max(captured_size);
    Ok(Some(RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::now(),
        process: process.clone(),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: payload_direction(event.direction),
        stream_key: PayloadStreamKey::new(format!("tls-sync:{}:{:x}", event.pid, event.stream_key)),
        sequence: event.sequence,
        original_size,
        captured_size,
        operation_id: event.sequence,
        operation_chunk_index: 0,
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
    process_cache: &mut BTreeMap<TlsSyncProcessCacheKey, ProcessObservation>,
) -> Result<Option<RawPayloadSegment>, ControlError> {
    let trace_id = TraceId::new(event.trace_id);
    let process = resolve_tls_sync_process(
        trace_id,
        event.pid,
        event.start_time_ticks,
        &event.pid_namespace,
        process_cache,
    )?;
    let captured_size = event.bytes.len() as u64;
    Ok(Some(RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::now(),
        process: process.clone(),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: payload_direction(event.direction),
        stream_key: PayloadStreamKey::new(format!("tls-sync:{}:{:x}", event.pid, event.stream_key)),
        sequence: event.sequence,
        original_size: captured_size,
        captured_size,
        operation_id: event.sequence,
        operation_chunk_index: 0,
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
    trace_id: TraceId,
    namespace_pid: u32,
    start_time_ticks: u64,
    pid_namespace: &str,
    process_cache: &mut BTreeMap<TlsSyncProcessCacheKey, ProcessObservation>,
) -> Result<ProcessObservation, ControlError> {
    let cache_key = TlsSyncProcessCacheKey {
        trace_id,
        namespace_pid,
        start_time_ticks,
        pid_namespace: pid_namespace.to_string(),
    };
    if let Some(process) = process_cache.get(&cache_key) {
        return Ok(process.clone());
    }
    if start_time_ticks == 0 || pid_namespace.is_empty() {
        return Err(ControlError::new(
            "tls_sync_pid_resolution",
            "TLS sync event process metadata is incomplete",
        ));
    }
    let process = ProcessObservation::namespace(NamespaceProcessCoordinates::new(
        NamespaceIdentity::new(pid_namespace),
        namespace_pid,
        start_time_ticks,
    ));
    process_cache.insert(cache_key, process.clone());
    Ok(process)
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TlsSyncProcessCacheKey {
    trace_id: TraceId,
    namespace_pid: u32,
    start_time_ticks: u64,
    pid_namespace: String,
}

fn payload_direction(direction: SyncDirection) -> PayloadDirection {
    match direction {
        SyncDirection::Outbound => PayloadDirection::Outbound,
        SyncDirection::Inbound => PayloadDirection::Inbound,
    }
}

fn create_parent_directory(socket_path: &Path) -> Result<(), ControlError> {
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

fn sync_event_error(error: tls_payload_sync::SyncError) -> ControlError {
    ControlError::new("tls_sync_event", error.to_string())
}

fn tls_flow_reason_code(reason: &str) -> i32 {
    match reason {
        "unknown_stream_threshold" => 1,
        "binary_unknown_stream" => 2,
        "http1_header_too_large" => 3,
        "large_non_text_transfer" => 4,
        "h2_binary_data" => 5,
        "h2_data_probe_exceeded" => 6,
        "flow_drop_discontinuity" => 7,
        "binary_body" => 8,
        _ => 0,
    }
}

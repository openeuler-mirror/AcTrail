//! Sync TLS payload event ingestion.

#[path = "clients.rs"]
mod clients;
#[path = "direct.rs"]
mod direct;
#[path = "frame_buffer.rs"]
mod frame_buffer;
#[path = "launch.rs"]
mod launch;
#[path = "limits.rs"]
mod limits;
#[path = "resolver.rs"]
mod resolver;
#[path = "root_path.rs"]
mod root_path;
pub(in crate::services) use direct::{DirectObject, DirectResolution};
pub(in crate::services) use launch::PinnedLaunchPlan;
pub(in crate::services) use limits::configure_fd_limit;

use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, SystemTime};

use config_core::daemon::{PayloadTlsCaptureBackend, PayloadTlsConfig};
use control_contract::reply::{ControlError, LaunchTlsPlanReply};
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::{NamespaceIdentity, NamespaceProcessCoordinates, ProcessObservation};
use payload_event::{RawPayloadSegment, RawPayloadStreamClose};
use storage_core::TlsFlowDiagnostic;
use tls_payload_core::PayloadDirection as SyncDirection;
use tls_payload_sync::{
    EventRegistration, FrameCodec, PayloadEvent, SummaryEvent, target_runtime_for_path,
};
use uds_control_server::PeerCredentials;

use self::clients::ReceiveLimits;
use self::frame_buffer::FrameBuffer;
use self::resolver::{ExecPlanConsumer, TlsSyncPlanResolver};
use self::root_path::{PeerRootHandle, PinnedPeerPath};

pub(crate) struct TlsSyncService {
    listener: Option<UnixListener>,
    clients: BTreeMap<u64, TlsSyncClient>,
    client_epoll: Option<OwnedFd>,
    accept_paused: bool,
    accept_retry_delay: Duration,
    client_max: usize,
    trace_connection_max: u32,
    trace_connections: BTreeMap<TraceId, TraceConnections>,
    next_client: u64,
    resolver: Option<TlsSyncPlanResolver>,
    read_buffer: Vec<u8>,
    receive_limits: ReceiveLimits,
    ready_events: Vec<libc::epoll_event>,
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
    pub(crate) payload_stream_closes: Vec<RawPayloadStreamClose>,
    pub(crate) diagnostics: Vec<TlsSyncDiagnostic>,
    pub(crate) flow_diagnostics: Vec<TlsFlowDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TlsSyncDiagnostic {
    pub(crate) trace_id: Option<TraceId>,
    pub(crate) code: String,
    pub(crate) message: String,
}

impl TlsSyncService {
    pub(crate) fn new(
        config: &PayloadTlsConfig,
        active_trace_max: u32,
    ) -> Result<Self, ControlError> {
        if !enabled(config) {
            let resolver = if config.enabled && config.capture_backend.uses_probe_plans() {
                Some(TlsSyncPlanResolver::new(config)?)
            } else {
                None
            };
            return Ok(Self {
                listener: None,
                clients: BTreeMap::new(),
                client_epoll: None,
                accept_paused: false,
                accept_retry_delay: Duration::from_millis(config.sync_accept_retry_delay_ms),
                client_max: 0,
                trace_connection_max: config.sync_trace_connection_max,
                trace_connections: BTreeMap::new(),
                next_client: 0,
                resolver,
                read_buffer: Vec::new(),
                receive_limits: ReceiveLimits {
                    max_frame_bytes: config.sync_max_frame_bytes as usize,
                    read_max_bytes: config.sync_read_max_bytes_per_client as usize,
                    buffer_retain_max_bytes: config.sync_frame_buffer_retain_max_bytes as usize,
                },
                ready_events: Vec::new(),
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
        let client_epoll = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if client_epoll < 0 {
            return Err(ControlError::new(
                "tls_sync_epoll",
                std::io::Error::last_os_error().to_string(),
            ));
        }
        let client_epoll = unsafe { OwnedFd::from_raw_fd(client_epoll) };
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
            clients: BTreeMap::new(),
            client_epoll: Some(client_epoll),
            accept_paused: false,
            accept_retry_delay: Duration::from_millis(config.sync_accept_retry_delay_ms),
            client_max: limits::client_limit(config, active_trace_max)?,
            trace_connection_max: config.sync_trace_connection_max,
            trace_connections: BTreeMap::new(),
            next_client: 0,
            resolver: Some(resolver),
            read_buffer: vec![0; config.sync_read_buffer_bytes as usize],
            receive_limits: ReceiveLimits {
                max_frame_bytes,
                read_max_bytes: config.sync_read_max_bytes_per_client as usize,
                buffer_retain_max_bytes: config.sync_frame_buffer_retain_max_bytes as usize,
            },
            ready_events: vec![
                libc::epoll_event { events: 0, u64: 0 };
                config.sync_ready_client_batch_max as usize
            ],
        })
    }

    pub(crate) fn event_poll_fds(&self) -> Vec<RawFd> {
        let mut fds = self
            .listener
            .as_ref()
            .filter(|_| !self.accept_paused)
            .map(AsRawFd::as_raw_fd)
            .into_iter()
            .collect::<Vec<_>>();
        fds.extend(self.client_epoll.as_ref().map(AsRawFd::as_raw_fd));
        fds.extend(
            self.resolver
                .as_ref()
                .and_then(TlsSyncPlanResolver::direct_poll_fd),
        );
        fds
    }

    pub(in crate::services) fn poll_timeout(&self) -> Option<Duration> {
        // Resolver replies can free descriptors without a client close event in this loop.
        self.accept_paused.then_some(self.accept_retry_delay)
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
        resolver.resolve_launch_plan(binary, PeerRootHandle::open(path_view_pid))
    }

    pub(crate) fn runtime_root_path_mapper(
        &self,
        path_view_pid: u32,
    ) -> Result<RuntimeRootPathMapper, ControlError> {
        PeerRootHandle::open(path_view_pid)
            .map(|root| RuntimeRootPathMapper { root })
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))
    }
}

struct TlsSyncClient {
    trace: Option<TraceId>,
    stream: UnixStream,
    credentials: PeerCredentials,
    registration: Option<EventRegistration>,
    buffer: FrameBuffer,
    process_cache: BTreeMap<TlsSyncProcessCacheKey, ProcessObservation>,
    /// Streams reported on this process transport, closed together when it disconnects.
    streams: BTreeMap<PayloadStreamKey, ProcessObservation>,
}

#[derive(Default)]
struct TraceConnections {
    active: u32,
    rejected: u64,
}

/// Names one TLS connection of a process. Payload segments and flow diagnostics of the same
/// connection carry the same key.
fn stream_key(pid: u32, connection: u64) -> String {
    format!("tls-sync:{pid}:{connection:x}")
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
    // Summary headers are metadata, not the next bytes of the TLS stream.
    // Preserve a zero-byte incomplete operation so consumers observe the gap.
    let captured_size = 0;
    let original_size = event.observed_size.max(captured_size);
    Ok(Some(RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::now(),
        process: process.clone(),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: payload_direction(event.direction),
        stream_key: PayloadStreamKey::new(stream_key(event.pid, event.stream_key)),
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
        bytes: Vec::new(),
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
        stream_key: PayloadStreamKey::new(stream_key(event.pid, event.stream_key)),
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

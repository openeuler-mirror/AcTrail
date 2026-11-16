//! Binary-analysis-cached TLS sync probe plan resolver.

#[path = "resolver_worker.rs"]
mod worker;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use super::direct::{DirectDiscovery, DirectObject, DirectResolution, DirectWorker};
use super::root_path::PeerRootHandle;
use config_core::daemon::{PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibraryPath};
use control_contract::reply::{
    ControlError, LaunchTlsPlanDescriptor, LaunchTlsPlanReply, LaunchTlsPlanStatus,
    LaunchTlsPlanUnavailableReason,
};
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use tls_payload_sync::{
    PlanLookupResponse, RuntimePlanDescriptor, encode_points, validate_native_backend_plan,
};
use tls_probe_point_finder::fast::{
    ArchFilter, FastProbeRequest, ProbeConsumer, ProviderFilter, SourceFilter,
};
use tls_probe_point_finder::{
    BinaryAnalysisCache, BinaryAnalysisCacheStats, BinaryIdentity, ProbeFileSource, ResolveMode,
    resolve_plans_with_analysis_cache, resolve_plans_with_file_source,
};

pub(super) struct TlsSyncPlanResolver {
    requests: SyncSender<WorkerRequest>,
    direct: Option<DirectDiscovery>,
    dynamic_exec_plan_timeout: Duration,
    launch_consumer: ProbeConsumer,
    startup_discovery_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExecPlanConsumer {
    Daemon,
    Sync,
}

struct PlanLookupJob {
    runtime_binary: PathBuf,
    consumer: ProbeConsumer,
    peer_root: Option<Result<PeerRootHandle, String>>,
    pin_peer_path: bool,
    response: Option<UnixStream>,
    control_response: Option<Sender<LaunchPlanLookupOutcome>>,
}

enum WorkerRequest {
    Lookup(PlanLookupJob),
    Direct(DirectObject),
}

struct TlsSyncPlanWorker {
    analysis_cache: Rc<BinaryAnalysisCache>,
    config: PayloadTlsConfig,
    match_limit: usize,
    direct: Option<DirectWorker>,
}

struct BinaryPlanRecord {
    plans: Vec<BinaryPlanDescriptor>,
}

struct BinaryPlanDescriptor {
    target: PathBuf,
    binary: PathBuf,
    target_identity: BinaryIdentity,
    binary_identity: BinaryIdentity,
    provider: String,
    source: String,
    points: String,
}

struct PlanLookupOutcome {
    response: PlanLookupResponse,
    launch_plans: Vec<LaunchTlsPlanDescriptor>,
    cache_hit: bool,
    elapsed: Duration,
}

struct LaunchPlanLookupOutcome {
    reply: Result<LaunchTlsPlanReply, ControlError>,
}

impl TlsSyncPlanResolver {
    pub(super) fn startup_discovery_enabled(&self) -> bool {
        self.startup_discovery_enabled
    }

    pub(super) fn new(config: &PayloadTlsConfig) -> Result<Self, ControlError> {
        let match_limit = match_limit(config)?;
        let cache_capacity = binary_analysis_cache_capacity(config)?;
        validate_library_candidates(config)?;
        let (requests, receiver) = mpsc::sync_channel(config.sync_plan_queue_capacity as usize);
        let (direct, direct_worker) = if config.capture_backend == PayloadTlsCaptureBackend::BpfCopy
            && config.direct_dynamic_discovery_enabled
        {
            let (discovery, worker) =
                DirectDiscovery::new(config.dynamic_discovery_capacity as usize)?;
            (Some(discovery), Some(worker))
        } else {
            (None, None)
        };
        let worker_config = config.clone();
        thread::Builder::new()
            .name("actrail-tls-plan-resolver".to_string())
            .spawn(move || {
                let analysis_cache = Rc::new(
                    BinaryAnalysisCache::new(cache_capacity)
                        .expect("validated TLS binary analysis cache capacity"),
                );
                TlsSyncPlanWorker {
                    analysis_cache,
                    config: worker_config,
                    match_limit,
                    direct: direct_worker,
                }
                .run(receiver);
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))?;
        Ok(Self {
            startup_discovery_enabled: config.enabled
                && config.capture_backend == PayloadTlsCaptureBackend::BpfCopy
                && config.direct_startup_discovery_enabled,
            requests,
            direct,
            dynamic_exec_plan_timeout: Duration::from_millis(config.dynamic_exec_plan_timeout_ms),
            launch_consumer: if config.capture_backend == PayloadTlsCaptureBackend::BpfCopy {
                ProbeConsumer::Direct
            } else {
                ProbeConsumer::Daemon
            },
        })
    }

    pub(super) fn direct_poll_fd(&self) -> Option<RawFd> {
        self.direct.as_ref().map(DirectDiscovery::poll_fd)
    }

    pub(super) fn direct_discovery_enabled(&self) -> bool {
        self.direct.is_some()
    }

    pub(super) fn submit_direct(&self, object: DirectObject) -> Option<DirectResolution> {
        self.direct.as_ref()?.submit(object, |object| {
            self.requests
                .try_send(WorkerRequest::Direct(object))
                .is_ok()
        })
    }

    pub(super) fn drain_direct(&mut self) -> Vec<DirectResolution> {
        self.direct
            .as_mut()
            .map(DirectDiscovery::drain)
            .unwrap_or_default()
    }

    pub(super) fn submit_lookup(
        &self,
        binary: &Path,
        peer_root: Result<PeerRootHandle, String>,
        response: UnixStream,
    ) -> Result<(), ControlError> {
        response
            .set_nonblocking(false)
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))?;
        match self.requests.try_send(WorkerRequest::Lookup(PlanLookupJob {
            runtime_binary: binary.to_path_buf(),
            consumer: ProbeConsumer::Sync,
            peer_root: Some(peer_root),
            pin_peer_path: false,
            response: Some(response),
            control_response: None,
        })) {
            // Dropping the request closes its socket. The runtime caches an Unsupported answer for
            // good, but treats a closed lookup like a timeout and asks again at the next load.
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(ControlError::new(
                "tls_sync_plan_worker",
                "TLS plan resolver stopped",
            )),
        }
    }

    pub(super) fn resolve_launch_plan(
        &self,
        binary: &Path,
        peer_root: Result<PeerRootHandle, String>,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        self.submit_control_lookup(binary, self.launch_consumer, Some(peer_root), true)?
            .recv()
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
            .and_then(|outcome| outcome.reply)
    }

    pub(super) fn resolve_exec_plan(
        &self,
        binary: &Path,
        consumer: ExecPlanConsumer,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        match self
            .submit_control_lookup(binary, consumer.probe_consumer(), None, false)?
            .recv_timeout(self.dynamic_exec_plan_timeout)
        {
            Ok(outcome) => outcome.reply,
            Err(RecvTimeoutError::Timeout) => Err(ControlError::new(
                "tls_sync_exec_plan_timeout",
                format!(
                    "TLS plan resolution for {} exceeded {} ms",
                    binary.display(),
                    self.dynamic_exec_plan_timeout.as_millis()
                ),
            )),
            Err(RecvTimeoutError::Disconnected) => Err(ControlError::new(
                "tls_sync_plan_worker",
                "TLS plan resolver stopped before returning the exec plan",
            )),
        }
    }

    fn submit_control_lookup(
        &self,
        binary: &Path,
        consumer: ProbeConsumer,
        peer_root: Option<Result<PeerRootHandle, String>>,
        pin_peer_path: bool,
    ) -> Result<Receiver<LaunchPlanLookupOutcome>, ControlError> {
        let (sender, receiver) = mpsc::channel();
        self.requests
            .try_send(WorkerRequest::Lookup(PlanLookupJob {
                runtime_binary: binary.to_path_buf(),
                consumer,
                peer_root,
                pin_peer_path,
                response: None,
                control_response: Some(sender),
            }))
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))?;
        Ok(receiver)
    }
}

impl ExecPlanConsumer {
    const fn probe_consumer(self) -> ProbeConsumer {
        match self {
            Self::Daemon => ProbeConsumer::Daemon,
            Self::Sync => ProbeConsumer::Sync,
        }
    }
}

fn outcome_for_record(
    record: BinaryPlanRecord,
    cache_hit: bool,
    started: Instant,
) -> PlanLookupOutcome {
    let mut launch_plans = Vec::with_capacity(record.plans.len());
    let mut response = None;
    for plan in record.plans {
        let descriptor = RuntimePlanDescriptor {
            target: plan.target,
            target_identity: plan.target_identity,
            binary: plan.binary,
            binary_identity: plan.binary_identity,
            provider: plan.provider,
            points: plan.points,
        };
        if response.is_none() {
            response = Some(PlanLookupResponse::Found(descriptor.clone()));
        }
        launch_plans.push(LaunchTlsPlanDescriptor {
            target: descriptor.target,
            target_identity: descriptor.target_identity,
            binary: descriptor.binary,
            binary_identity: descriptor.binary_identity,
            provider: descriptor.provider,
            source: plan.source,
            points: descriptor.points,
        });
    }
    PlanLookupOutcome {
        response: response.expect("Found record has at least one plan"),
        launch_plans,
        cache_hit,
        elapsed: started.elapsed(),
    }
}

fn unsupported_outcome(reason: String, started: Instant) -> PlanLookupOutcome {
    PlanLookupOutcome {
        response: PlanLookupResponse::Unsupported { reason },
        launch_plans: Vec::new(),
        cache_hit: false,
        elapsed: started.elapsed(),
    }
}

fn launch_reply_for_outcome(outcome: PlanLookupOutcome) -> LaunchTlsPlanReply {
    let status = if outcome.launch_plans.is_empty() {
        LaunchTlsPlanStatus::Unsupported {
            reason: LaunchTlsPlanUnavailableReason::AnalysisRejected,
        }
    } else {
        LaunchTlsPlanStatus::Found(outcome.launch_plans)
    };
    LaunchTlsPlanReply {
        status,
        cache_hit: outcome.cache_hit,
        resolve_elapsed_micros: duration_micros(outcome.elapsed),
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn runtime_view_binary(plan_binary: &Path, runtime_binary: &Path, probe_binary: &Path) -> PathBuf {
    if plan_binary == probe_binary {
        return runtime_binary.to_path_buf();
    }
    proc_root_runtime_path(plan_binary).unwrap_or_else(|| plan_binary.to_path_buf())
}

fn proc_root_runtime_path(path: &Path) -> Option<PathBuf> {
    let raw = path.as_os_str().to_string_lossy();
    let (_, suffix) = raw.strip_prefix("/proc/")?.split_once("/root/")?;
    Some(Path::new("/").join(suffix))
}

fn library_candidates(config: &PayloadTlsConfig) -> Vec<PathBuf> {
    match &config.library_path {
        PayloadTlsLibraryPath::Auto => Vec::new(),
        PayloadTlsLibraryPath::Path(path) => vec![path.clone()],
    }
}

fn match_limit(config: &PayloadTlsConfig) -> Result<usize, ControlError> {
    usize::try_from(config.sync_match_limit).map_err(|error| {
        ControlError::new(
            "tls_sync_config",
            format!("payload_tls_sync_match_limit overflow: {error}"),
        )
    })
}

fn binary_analysis_cache_capacity(config: &PayloadTlsConfig) -> Result<usize, ControlError> {
    let capacity = usize::try_from(config.binary_analysis_cache_capacity).map_err(|error| {
        ControlError::new(
            "tls_sync_config",
            format!("payload_tls_binary_analysis_cache_capacity overflow: {error}"),
        )
    })?;
    if capacity == 0 {
        return Err(ControlError::new(
            "tls_sync_config",
            "payload_tls_binary_analysis_cache_capacity must be greater than zero",
        ));
    }
    Ok(capacity)
}

const fn probe_consumer_name(consumer: ProbeConsumer) -> &'static str {
    match consumer {
        ProbeConsumer::PlanOnly => "plan-only",
        ProbeConsumer::Standalone => "standalone",
        ProbeConsumer::Sync => "sync",
        ProbeConsumer::Daemon => "daemon",
        ProbeConsumer::Direct => "direct",
    }
}

fn validate_library_candidates(config: &PayloadTlsConfig) -> Result<(), ControlError> {
    for path in library_candidates(config) {
        if !path.is_file() {
            return Err(ControlError::new(
                "tls_sync_config",
                format!("payload_tls_library_path is not a file: {}", path.display()),
            ));
        }
    }
    Ok(())
}

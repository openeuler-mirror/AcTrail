//! Binary-analysis-cached TLS sync probe plan resolver.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
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
use tls_payload_sync::{
    PlanLookupResponse, RuntimePlanDescriptor, encode_points, validate_native_backend_plan,
};
use tls_probe_point_finder::fast::{
    ArchFilter, FastProbeRequest, ProbeConsumer, ProviderFilter, SourceFilter,
};
use tls_probe_point_finder::{
    BinaryAnalysisCache, BinaryAnalysisCacheStats, BinaryIdentity, ResolveMode,
    resolve_plans_with_analysis_cache,
};

pub(super) struct TlsSyncPlanResolver {
    requests: Sender<WorkerRequest>,
    direct: Option<DirectDiscovery>,
    dynamic_exec_plan_timeout: Duration,
    launch_consumer: ProbeConsumer,
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
    pub(super) fn new(config: &PayloadTlsConfig) -> Result<Self, ControlError> {
        let match_limit = match_limit(config)?;
        let cache_capacity = binary_analysis_cache_capacity(config)?;
        validate_library_candidates(config)?;
        let (requests, receiver) = mpsc::channel();
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
            self.requests.send(WorkerRequest::Direct(object)).is_ok()
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
        self.requests
            .send(WorkerRequest::Lookup(PlanLookupJob {
                runtime_binary: binary.to_path_buf(),
                consumer: ProbeConsumer::Sync,
                peer_root: Some(peer_root),
                pin_peer_path: false,
                response: Some(response),
                control_response: None,
            }))
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
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
            .send(WorkerRequest::Lookup(PlanLookupJob {
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

impl TlsSyncPlanWorker {
    fn run(mut self, receiver: Receiver<WorkerRequest>) {
        for request in receiver {
            let mut job = match request {
                WorkerRequest::Lookup(job) => job,
                WorkerRequest::Direct(object) => {
                    if let Some(worker) = &mut self.direct {
                        worker.resolve(object, &self.analysis_cache, self.match_limit);
                    }
                    continue;
                }
            };
            if job.consumer == ProbeConsumer::Direct {
                let reply = self.lookup_direct(&job.runtime_binary, job.peer_root);
                if let Some(sender) = job.control_response {
                    let _ = sender.send(LaunchPlanLookupOutcome { reply });
                }
                continue;
            }
            let outcome = self.lookup(
                &job.runtime_binary,
                job.consumer,
                job.peer_root,
                job.pin_peer_path,
            );
            let Some(response_stream) = job.response.as_mut() else {
                if let Some(sender) = job.control_response {
                    let _ = sender.send(LaunchPlanLookupOutcome {
                        reply: Ok(launch_reply_for_outcome(outcome)),
                    });
                }
                continue;
            };
            if let Err(error) = tls_payload_sync::FrameCodec::write_lookup_response(
                response_stream,
                &outcome.response,
                self.config.sync_max_frame_bytes as usize,
            ) {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    binary = %job.runtime_binary.display(),
                    error = %error,
                    "failed to write TLS sync plan lookup response"
                );
            }
        }
    }

    fn lookup_direct(
        &mut self,
        runtime_binary: &Path,
        peer_root: Option<Result<PeerRootHandle, String>>,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        let started = Instant::now();
        let root = peer_root
            .ok_or_else(|| ControlError::new("tls_sync_plan_root", ""))?
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))?;
        let pinned = root
            .pin_path(runtime_binary)
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))?;
        let startup_object = DirectObject::open_startup(&pinned.path());
        let before = self.analysis_cache.stats();
        let resolved = self.resolve_plans(&pinned.path(), runtime_binary, ProbeConsumer::Direct);
        if let (Some(worker), Some(object), Ok(plans)) = (&self.direct, startup_object, &resolved) {
            // A successful Auto/All pass completed the executable branches.
            // Only the pinned root's facts belong in its single-object cache entry.
            let root_plans = plans.iter().filter(|plan| {
                plan.source == "executable"
                    && plan.target == runtime_binary
                    && plan.binary == runtime_binary
            });
            if root_plans
                .clone()
                .all(|plan| matches!(plan.provider.as_str(), "openssl" | "rustls"))
            {
                worker.seed_startup(
                    object,
                    root_plans.map(|plan| {
                        (
                            plan.binary_identity.clone(),
                            plan.provider.clone(),
                            plan.points.clone(),
                        )
                    }),
                );
            }
        }
        let status = match resolved {
            Ok(plans) if plans.is_empty() => LaunchTlsPlanStatus::Unsupported {
                reason: LaunchTlsPlanUnavailableReason::NoProbePoints,
            },
            Ok(plans) => LaunchTlsPlanStatus::Found(
                plans
                    .into_iter()
                    .map(|plan| LaunchTlsPlanDescriptor {
                        target: plan.target,
                        binary: plan.binary,
                        target_identity: plan.target_identity,
                        binary_identity: plan.binary_identity,
                        provider: plan.provider,
                        source: plan.source,
                        points: plan.points,
                    })
                    .collect(),
            ),
            Err(_) => LaunchTlsPlanStatus::Unsupported {
                reason: LaunchTlsPlanUnavailableReason::AnalysisRejected,
            },
        };
        let after = self.analysis_cache.stats();
        Ok(LaunchTlsPlanReply {
            status,
            cache_hit: after.misses == before.misses && after.hits > before.hits,
            resolve_elapsed_micros: duration_micros(started.elapsed()),
        })
    }

    fn lookup(
        &mut self,
        runtime_binary: &Path,
        consumer: ProbeConsumer,
        peer_root: Option<Result<PeerRootHandle, String>>,
        pin_peer_path: bool,
    ) -> PlanLookupOutcome {
        let started = Instant::now();
        let peer_root = match peer_root {
            Some(Ok(root)) => Some(root),
            Some(Err(reason)) => {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    runtime_binary = %runtime_binary.display(),
                    reason = %reason,
                    "TLS sync plan lookup path resolution failed"
                );
                return unsupported_outcome(reason, started);
            }
            None => None,
        };
        let pinned_probe = if pin_peer_path {
            match peer_root.as_ref() {
                Some(root) => match root.pin_path(runtime_binary) {
                    Ok(path) => Some(path),
                    Err(reason) => {
                        tracing::warn!(
                            target: "actrail::tls_sync",
                            runtime_binary = %runtime_binary.display(),
                            reason = %reason,
                            "TLS sync plan lookup path resolution failed"
                        );
                        return unsupported_outcome(reason, started);
                    }
                },
                None => {
                    return unsupported_outcome(
                        "authenticated peer root is required for pinned TLS plan lookup"
                            .to_string(),
                        started,
                    );
                }
            }
        } else {
            None
        };
        let probe_binary = match pinned_probe.as_ref() {
            Some(path) => path.path(),
            None => match probe_binary_path(runtime_binary, peer_root.as_ref()) {
                Ok(path) => path,
                Err(reason) => {
                    tracing::warn!(
                        target: "actrail::tls_sync",
                        runtime_binary = %runtime_binary.display(),
                        reason = %reason,
                        "TLS sync plan lookup path resolution failed"
                    );
                    return unsupported_outcome(reason, started);
                }
            },
        };
        let cache_before = self.analysis_cache.stats();
        let record = match self.resolve_plans(&probe_binary, runtime_binary, consumer) {
            Ok(plans) => BinaryPlanRecord { plans },
            Err(error) => {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    runtime_binary = %runtime_binary.display(),
                    probe_binary = %probe_binary.display(),
                    error = %error.message,
                    "TLS sync plan lookup probe failed"
                );
                return unsupported_outcome(error.message, started);
            }
        };
        let cache_after = self.analysis_cache.stats();
        let cache_hit =
            cache_after.misses == cache_before.misses && cache_after.hits > cache_before.hits;
        self.log_cache_lookup(
            consumer,
            runtime_binary,
            &probe_binary,
            cache_hit,
            cache_after,
        );
        outcome_for_record(record, cache_hit, started)
    }

    fn resolve_plans(
        &self,
        probe_binary: &Path,
        runtime_binary: &Path,
        consumer: ProbeConsumer,
    ) -> Result<Vec<BinaryPlanDescriptor>, ControlError> {
        let resolution = resolve_plans_with_analysis_cache(
            FastProbeRequest {
                binary: probe_binary.to_path_buf(),
                arch: ArchFilter::Auto,
                provider: ProviderFilter::Auto,
                source: SourceFilter::Auto,
                match_limit: self.match_limit,
                libraries: library_candidates(&self.config),
                library_search_dirs: Vec::new(),
            },
            consumer,
            ResolveMode::All,
            Rc::clone(&self.analysis_cache),
        )
        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
        if resolution.plans.is_empty() && consumer != ProbeConsumer::Direct {
            return Err(ControlError::new(
                "tls_sync_plan",
                "no supported TLS payload probe points found",
            ));
        }
        resolution
            .plans
            .into_iter()
            .map(|plan| {
                if consumer != ProbeConsumer::Direct {
                    validate_native_backend_plan(&plan)
                        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
                }
                Ok(BinaryPlanDescriptor {
                    target: runtime_view_binary(&plan.target.binary, runtime_binary, probe_binary),
                    binary: runtime_view_binary(&plan.binary.path, runtime_binary, probe_binary),
                    target_identity: plan.target.identity.clone(),
                    binary_identity: plan.binary.identity.clone(),
                    provider: plan.provider.as_str().to_string(),
                    source: plan.source.as_str().to_string(),
                    points: encode_points(&plan)
                        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?,
                })
            })
            .collect()
    }

    fn log_cache_lookup(
        &self,
        consumer: ProbeConsumer,
        runtime_binary: &Path,
        probe_binary: &Path,
        cache_hit: bool,
        stats: BinaryAnalysisCacheStats,
    ) {
        if !self.config.diagnostics_enabled {
            return;
        }
        tracing::info!(
            target: "actrail::tls_sync",
            consumer = probe_consumer_name(consumer),
            runtime_binary = %runtime_binary.display(),
            probe_binary = %probe_binary.display(),
            cache = if cache_hit { "hit" } else { "miss" },
            cache_entries = stats.entries,
            cache_evictions = stats.evictions,
            "TLS binary analysis cache lookup"
        );
    }
}

fn probe_binary_path(
    runtime_binary: &Path,
    peer_root: Option<&PeerRootHandle>,
) -> Result<PathBuf, String> {
    match peer_root {
        Some(root) => root.probe_path_for(runtime_binary),
        None => Ok(runtime_binary.to_path_buf()),
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

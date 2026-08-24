//! Binary-analysis-cached TLS sync probe plan resolver.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::root_path::PeerRootHandle;
use config_core::daemon::{PayloadTlsConfig, PayloadTlsLibraryPath};
use control_contract::reply::{
    ControlError, LaunchTlsPlanDescriptor, LaunchTlsPlanReply, LaunchTlsPlanStatus,
};
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
    requests: Sender<PlanLookupJob>,
    dynamic_exec_plan_timeout: Duration,
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
    response: Option<UnixStream>,
    control_response: Option<Sender<LaunchPlanLookupOutcome>>,
}

struct TlsSyncPlanWorker {
    analysis_cache: Rc<BinaryAnalysisCache>,
    config: PayloadTlsConfig,
    match_limit: usize,
}

struct BinaryPlanRecord {
    plans: Vec<BinaryPlanDescriptor>,
}

struct BinaryPlanDescriptor {
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
    reply: LaunchTlsPlanReply,
}

impl TlsSyncPlanResolver {
    pub(super) fn new(config: &PayloadTlsConfig) -> Result<Self, ControlError> {
        let match_limit = match_limit(config)?;
        let cache_capacity = binary_analysis_cache_capacity(config)?;
        validate_library_candidates(config)?;
        let (requests, receiver) = mpsc::channel();
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
                }
                .run(receiver);
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))?;
        Ok(Self {
            requests,
            dynamic_exec_plan_timeout: Duration::from_millis(config.dynamic_exec_plan_timeout_ms),
        })
    }

    pub(super) fn submit_lookup(
        &self,
        binary: &Path,
        peer_root: Result<PeerRootHandle, String>,
        response: UnixStream,
    ) -> Result<(), ControlError> {
        self.requests
            .send(PlanLookupJob {
                runtime_binary: binary.to_path_buf(),
                consumer: ProbeConsumer::Sync,
                peer_root: Some(peer_root),
                response: Some(response),
                control_response: None,
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }

    pub(super) fn resolve_launch_plan(
        &self,
        binary: &Path,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        self.submit_control_lookup(binary, ProbeConsumer::Daemon)?
            .recv()
            .map(|outcome| outcome.reply)
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }

    pub(super) fn resolve_exec_plan(
        &self,
        binary: &Path,
        consumer: ExecPlanConsumer,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        match self
            .submit_control_lookup(binary, consumer.probe_consumer())?
            .recv_timeout(self.dynamic_exec_plan_timeout)
        {
            Ok(outcome) => Ok(outcome.reply),
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
    ) -> Result<Receiver<LaunchPlanLookupOutcome>, ControlError> {
        let (sender, receiver) = mpsc::channel();
        self.requests
            .send(PlanLookupJob {
                runtime_binary: binary.to_path_buf(),
                consumer,
                peer_root: None,
                response: None,
                control_response: Some(sender),
            })
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
    fn run(mut self, receiver: Receiver<PlanLookupJob>) {
        for mut job in receiver {
            let outcome = self.lookup(&job.runtime_binary, job.consumer, job.peer_root);
            let Some(response_stream) = job.response.as_mut() else {
                if let Some(sender) = job.control_response {
                    let _ = sender.send(LaunchPlanLookupOutcome {
                        reply: launch_reply_for_outcome(outcome),
                    });
                }
                continue;
            };
            if let Err(error) = response_stream.write_all(
                &tls_payload_sync::encode_plan_lookup_response(&outcome.response),
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

    fn lookup(
        &mut self,
        runtime_binary: &Path,
        consumer: ProbeConsumer,
        peer_root: Option<Result<PeerRootHandle, String>>,
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
        let probe_binary = match probe_binary_path(runtime_binary, peer_root.as_ref()) {
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
        outcome_for_record(record, runtime_binary, cache_hit, started)
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
        if resolution.plans.is_empty() {
            return Err(ControlError::new(
                "tls_sync_plan",
                "no supported TLS payload probe points found",
            ));
        }
        resolution
            .plans
            .into_iter()
            .map(|plan| {
                validate_native_backend_plan(&plan)
                    .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
                Ok(BinaryPlanDescriptor {
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
    runtime_binary: &Path,
    cache_hit: bool,
    started: Instant,
) -> PlanLookupOutcome {
    let mut launch_plans = Vec::with_capacity(record.plans.len());
    let mut response = None;
    for plan in record.plans {
        let descriptor = RuntimePlanDescriptor {
            target: runtime_binary.to_path_buf(),
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
        let reason = match outcome.response {
            PlanLookupResponse::Unsupported { reason } => reason,
            PlanLookupResponse::Found(_) => "empty TLS plan set".to_string(),
        };
        LaunchTlsPlanStatus::Unsupported { reason }
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

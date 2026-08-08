//! Store-backed TLS sync probe plan resolver.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

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
use tls_probe_point_finder::{ResolveMode, resolve_plans};

use super::plan_store::{
    BinaryPlanDescriptor, BinaryPlanKey, BinaryPlanRecord, BinaryPlanStore, InMemoryBinaryPlanStore,
};
use super::root_path::PeerRootHandle;

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
    store: Box<dyn BinaryPlanStore + Send>,
    config: PayloadTlsConfig,
    match_limit: usize,
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
        validate_library_candidates(config)?;
        let (requests, receiver) = mpsc::channel();
        let worker = TlsSyncPlanWorker {
            store: Box::<InMemoryBinaryPlanStore>::default(),
            config: config.clone(),
            match_limit,
        };
        thread::Builder::new()
            .name("actrail-tls-plan-resolver".to_string())
            .spawn(move || worker.run(receiver))
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
        let key = match BinaryPlanKey::for_path(&probe_binary, consumer) {
            Ok(key) => key,
            Err(error) => {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    runtime_binary = %runtime_binary.display(),
                    probe_binary = %probe_binary.display(),
                    error = %error,
                    "TLS sync plan lookup probe binary stat failed"
                );
                return unsupported_outcome(
                    format!(
                        "stat probe binary runtime={} probe={}: {error}",
                        runtime_binary.display(),
                        probe_binary.display()
                    ),
                    started,
                );
            }
        };
        match self.store.get(&key) {
            Ok(Some(cached)) => {
                return outcome_for_record(cached, runtime_binary, &probe_binary, true, started);
            }
            Ok(None) => {}
            Err(error) => {
                return unsupported_outcome(
                    format!("load cached probe plan {}: {error}", key.path().display()),
                    started,
                );
            }
        }
        let cached = match self.resolve_plans(key.path(), consumer) {
            Ok(plan) => BinaryPlanRecord::Found(plan),
            Err(error) => {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    runtime_binary = %runtime_binary.display(),
                    probe_binary = %probe_binary.display(),
                    error = %error.message,
                    "TLS sync plan lookup probe failed"
                );
                BinaryPlanRecord::Unsupported(error.message)
            }
        };
        let outcome = outcome_for_record(
            cached.clone(),
            runtime_binary,
            &probe_binary,
            false,
            started,
        );
        if let Err(error) = self.store.put(key, cached) {
            return unsupported_outcome(format!("store probe plan: {error}"), started);
        }
        outcome
    }

    fn resolve_plans(
        &self,
        binary: &Path,
        consumer: ProbeConsumer,
    ) -> Result<Vec<BinaryPlanDescriptor>, ControlError> {
        let resolution = resolve_plans(
            FastProbeRequest {
                binary: binary.to_path_buf(),
                arch: ArchFilter::Auto,
                provider: ProviderFilter::Auto,
                source: SourceFilter::Auto,
                match_limit: self.match_limit,
                libraries: library_candidates(&self.config),
                library_search_dirs: Vec::new(),
            },
            consumer,
            ResolveMode::All,
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
                    binary: plan.binary.path.clone(),
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
    probe_binary: &Path,
    cache_hit: bool,
    started: Instant,
) -> PlanLookupOutcome {
    match record {
        BinaryPlanRecord::Found(plans) => {
            let mut launch_plans = Vec::with_capacity(plans.len());
            let mut response = None;
            for plan in plans {
                let descriptor = RuntimePlanDescriptor {
                    target: runtime_binary.to_path_buf(),
                    target_identity: plan.target_identity,
                    binary: runtime_view_binary(&plan.binary, runtime_binary, probe_binary),
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
        BinaryPlanRecord::Unsupported(reason) => PlanLookupOutcome {
            response: PlanLookupResponse::Unsupported { reason },
            launch_plans: Vec::new(),
            cache_hit,
            elapsed: started.elapsed(),
        },
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

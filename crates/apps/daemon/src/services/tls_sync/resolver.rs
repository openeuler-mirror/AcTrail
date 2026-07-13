//! Store-backed TLS sync probe plan resolver.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
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
    ArchFilter, FastProbeRequest, ProviderFilter, SourceFilter, resolve,
};

use super::plan_store::{
    BinaryPlanDescriptor, BinaryPlanKey, BinaryPlanRecord, BinaryPlanStore, InMemoryBinaryPlanStore,
};
use super::root_path::PeerRootHandle;

pub(super) struct TlsSyncPlanResolver {
    requests: Sender<PlanLookupJob>,
}

struct PlanLookupJob {
    runtime_binary: PathBuf,
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
    cache_hit: bool,
    elapsed: Duration,
    source: Option<String>,
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
        Ok(Self { requests })
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
                peer_root: Some(peer_root),
                response: Some(response),
                control_response: None,
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }

    pub(super) fn prewarm(&self, binary: &Path) -> Result<(), ControlError> {
        self.requests
            .send(PlanLookupJob {
                runtime_binary: binary.to_path_buf(),
                peer_root: None,
                response: None,
                control_response: None,
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }

    pub(super) fn resolve_launch_plan(
        &self,
        binary: &Path,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        let (sender, receiver) = mpsc::channel();
        self.requests
            .send(PlanLookupJob {
                runtime_binary: binary.to_path_buf(),
                peer_root: None,
                response: None,
                control_response: Some(sender),
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))?;
        receiver
            .recv()
            .map(|outcome| outcome.reply)
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }
}

impl TlsSyncPlanWorker {
    fn run(mut self, receiver: Receiver<PlanLookupJob>) {
        for mut job in receiver {
            let outcome = self.lookup(&job.runtime_binary, job.peer_root);
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
        let key = match BinaryPlanKey::for_path(&probe_binary) {
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
        let cached = match self.resolve_plan(key.path()) {
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

    fn resolve_plan(&self, binary: &Path) -> Result<BinaryPlanDescriptor, ControlError> {
        let plan = resolve(FastProbeRequest {
            binary: binary.to_path_buf(),
            arch: ArchFilter::Auto,
            provider: ProviderFilter::Auto,
            source: SourceFilter::Auto,
            match_limit: self.match_limit,
            libraries: library_candidates(&self.config),
            library_search_dirs: Vec::new(),
        })
        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
        validate_native_backend_plan(&plan)
            .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
        Ok(BinaryPlanDescriptor {
            binary: plan.binary.path.clone(),
            provider: plan.provider.as_str().to_string(),
            source: plan.source.as_str().to_string(),
            points: encode_points(&plan)
                .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?,
        })
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
        BinaryPlanRecord::Found(plan) => {
            let source = plan.source.clone();
            PlanLookupOutcome {
                response: PlanLookupResponse::Found(RuntimePlanDescriptor {
                    target: runtime_binary.to_path_buf(),
                    binary: runtime_view_binary(&plan.binary, runtime_binary, probe_binary),
                    provider: plan.provider,
                    points: plan.points,
                }),
                cache_hit,
                elapsed: started.elapsed(),
                source: Some(source),
            }
        }
        BinaryPlanRecord::Unsupported(reason) => PlanLookupOutcome {
            response: PlanLookupResponse::Unsupported { reason },
            cache_hit,
            elapsed: started.elapsed(),
            source: None,
        },
    }
}

fn unsupported_outcome(reason: String, started: Instant) -> PlanLookupOutcome {
    PlanLookupOutcome {
        response: PlanLookupResponse::Unsupported { reason },
        cache_hit: false,
        elapsed: started.elapsed(),
        source: None,
    }
}

fn launch_reply_for_outcome(outcome: PlanLookupOutcome) -> LaunchTlsPlanReply {
    let status = match outcome.response {
        PlanLookupResponse::Found(plan) => LaunchTlsPlanStatus::Found(LaunchTlsPlanDescriptor {
            target: plan.target,
            binary: plan.binary,
            provider: plan.provider,
            source: outcome.source.unwrap_or_else(|| "unknown".to_string()),
            points: plan.points,
        }),
        PlanLookupResponse::Unsupported { reason } => LaunchTlsPlanStatus::Unsupported { reason },
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

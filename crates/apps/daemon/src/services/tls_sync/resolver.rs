//! Store-backed TLS sync probe plan resolver.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use config_core::daemon::{PayloadTlsConfig, PayloadTlsLibraryPath};
use control_contract::reply::ControlError;
use tls_payload_sync::{
    PlanLookupResponse, RuntimePlanDescriptor, runtime_plan_descriptor,
    validate_native_backend_plan,
};
use tls_probe_point_finder::fast::{
    ArchFilter, FastProbeRequest, ProviderFilter, SourceFilter, resolve,
};

use super::plan_store::{
    BinaryPlanKey, BinaryPlanRecord, BinaryPlanStore, InMemoryBinaryPlanStore,
};

pub(super) struct TlsSyncPlanResolver {
    requests: Sender<PlanLookupJob>,
}

struct PlanLookupJob {
    binary: PathBuf,
    response: Option<UnixStream>,
}

struct TlsSyncPlanWorker {
    store: Box<dyn BinaryPlanStore + Send>,
    config: PayloadTlsConfig,
    match_limit: usize,
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
        response: UnixStream,
    ) -> Result<(), ControlError> {
        self.requests
            .send(PlanLookupJob {
                binary: binary.to_path_buf(),
                response: Some(response),
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }

    pub(super) fn prewarm(&self, binary: &Path) -> Result<(), ControlError> {
        self.requests
            .send(PlanLookupJob {
                binary: binary.to_path_buf(),
                response: None,
            })
            .map_err(|error| ControlError::new("tls_sync_plan_worker", error.to_string()))
    }
}

impl TlsSyncPlanWorker {
    fn run(mut self, receiver: Receiver<PlanLookupJob>) {
        for mut job in receiver {
            let response = self.lookup(&job.binary);
            let Some(response_stream) = job.response.as_mut() else {
                continue;
            };
            if let Err(error) =
                response_stream.write_all(&tls_payload_sync::encode_plan_lookup_response(&response))
            {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    binary = %job.binary.display(),
                    error = %error,
                    "failed to write TLS sync plan lookup response"
                );
            }
        }
    }

    fn lookup(&mut self, binary: &Path) -> PlanLookupResponse {
        let key = match BinaryPlanKey::for_path(binary) {
            Ok(key) => key,
            Err(error) => {
                return PlanLookupResponse::Unsupported {
                    reason: format!("stat probe binary {}: {error}", binary.display()),
                };
            }
        };
        match self.store.get(&key) {
            Ok(Some(cached)) => return cached.response(),
            Ok(None) => {}
            Err(error) => {
                return PlanLookupResponse::Unsupported {
                    reason: format!("load cached probe plan {}: {error}", key.path().display()),
                };
            }
        }
        let cached = match self.resolve_plan(key.path()) {
            Ok(plan) => BinaryPlanRecord::Found(plan),
            Err(error) => BinaryPlanRecord::Unsupported(error.message),
        };
        let response = cached.response();
        if let Err(error) = self.store.put(key, cached) {
            return PlanLookupResponse::Unsupported {
                reason: format!("store probe plan: {error}"),
            };
        }
        response
    }

    fn resolve_plan(&self, binary: &Path) -> Result<RuntimePlanDescriptor, ControlError> {
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
        runtime_plan_descriptor(&plan)
            .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))
    }
}

impl BinaryPlanRecord {
    fn response(&self) -> PlanLookupResponse {
        match self {
            Self::Found(plan) => PlanLookupResponse::Found(plan.clone()),
            Self::Unsupported(reason) => PlanLookupResponse::Unsupported {
                reason: reason.clone(),
            },
        }
    }
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

use super::super::cgroup_v2_backend_factory::CgroupSamplerFactory;
use crate::{CgroupV2Backend, CgroupWorkloadSample, ResourceProfilingBackend, SamplingContext};
// Managed cgroup v2 initialization and startup reconciliation.

use std::collections::{BTreeMap, BTreeSet};

use super::{ManagedRecoveryRecord, ManagedScopePersistence};
use crate::ProfilingError;
use crate::model::resource_scope::TraceResourceScope;
use crate::{ManagedScopeConfig, WorkloadBackendMode as ResourceMetricsMode};
use linux_cgroup::{
    CgroupPreflightReport, ManagedCgroupV2, TraceScopePaths, discover_unified_hierarchy,
};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

pub(crate) struct ManagedScopeManager {
    adapter: ManagedCgroupV2,
    preflight: CgroupPreflightReport,
    controllers: Vec<String>,
    scopes: BTreeMap<TraceId, TraceScopePaths>,
    waiting_since: BTreeMap<TraceId, std::time::SystemTime>,
    finalized_barriers: BTreeSet<TraceId>,
    recovered_processes: BTreeMap<TraceId, ProcessIdentity>,
    readers: BTreeMap<TraceId, CgroupV2Backend<linux_cgroup::CgroupV2CounterReader>>,
    backend_factory: CgroupSamplerFactory,
    pending_admissions: BTreeMap<TraceId, TraceScopePaths>,
    pending_cleanup: BTreeMap<std::path::PathBuf, TraceScopePaths>,
    recovery: ResourceScopeRecovery,
}

pub(crate) enum CgroupAdmission {
    Managed(TraceResourceScope),
    Fallback(String),
}

use super::scope_recovery::ResourceScopeRecovery;

impl ManagedScopeManager {
    pub(crate) fn initialize(
        config: &ManagedScopeConfig,
        backend_factory: CgroupSamplerFactory,
        storage: &mut dyn ManagedScopePersistence,
    ) -> Result<(Option<Self>, Option<String>), ProfilingError> {
        // Storage errors are never an availability fallback: without durable
        // state we cannot prove that there is no managed scope to recover.
        let records = storage.prepare_recovery()?;
        let recovery_required = records
            .iter()
            .any(|record| record.scope.lifecycle_state.is_live());
        match Self::initialize_required(config, backend_factory, storage, records) {
            Ok(runtime) => {
                tracing::info!(
                    managed_root = %runtime.preflight.managed_root.display(),
                    owner_uid = runtime.preflight.owner_uid,
                    owner_gid = runtime.preflight.owner_gid,
                    controllers = ?runtime.controllers,
                    recovered_scopes = runtime.scopes.len(),
                    registered_missing = runtime.recovery.registered_missing.len(),
                    unregistered_empty = runtime.recovery.unregistered_empty.len(),
                    unregistered_populated = runtime.recovery.unregistered_populated.len(),
                    terminal_empty = runtime.recovery.terminal_empty.len(),
                    terminal_populated = runtime.recovery.terminal_populated.len(),
                    unknown_directories = runtime.recovery.unknown_directories.len(),
                    "cgroup v2 resource metrics preflight completed"
                );
                Ok((Some(runtime), None))
            }
            Err(error) if !recovery_required && error.is_unavailable() => {
                let reason = format!("{}: {}", error.code, error.message);
                tracing::warn!(
                    reason = %reason,
                    "managed cgroup admission unavailable"
                );
                Ok((None, Some(reason)))
            }
            Err(error) => Err(error),
        }
    }

    fn initialize_required(
        config: &ManagedScopeConfig,
        backend_factory: CgroupSamplerFactory,
        storage: &mut dyn ManagedScopePersistence,
        records: Vec<ManagedRecoveryRecord>,
    ) -> Result<Self, ProfilingError> {
        let hierarchy =
            discover_unified_hierarchy().map_err(Self::cgroup_error("cgroup_discovery"))?;
        let adapter = ManagedCgroupV2::new(config.root.clone())
            .map_err(Self::cgroup_error("cgroup_configuration"))?;
        let preflight = adapter
            .prepare_hierarchy(hierarchy, std::process::id())
            .map_err(Self::cgroup_error("cgroup_preflight"))?;
        let controllers = preflight
            .enabled_controllers
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let scan_limit = usize::try_from(config.recovery_scan_limit).map_err(|error| {
            ProfilingError::new("cgroup_recovery", format!("orphan limit overflow: {error}"))
        })?;
        let scan = adapter
            .scan_managed_scopes(scan_limit)
            .map_err(Self::cgroup_error("cgroup_recovery"))?;
        let roots = records
            .iter()
            .map(|record| (record.scope.trace_id, record.root))
            .collect::<BTreeMap<_, _>>();
        let registered = records.into_iter().map(|record| record.scope).collect();
        let (scopes, waiting_since, finalized_barriers, recovery) =
            ResourceScopeRecovery::reconcile(&adapter, registered, scan)?;
        let recovered_processes = roots
            .into_iter()
            .filter(|(id, _)| scopes.contains_key(id))
            .collect();
        let mut pending_cleanup = BTreeMap::new();
        for directory in recovery
            .unregistered_empty
            .iter()
            .chain(recovery.terminal_empty.iter())
        {
            if let Err(error) = adapter.remove_empty_trace_scope(&directory.paths) {
                tracing::warn!(
                    trace_id = directory.trace_id,
                    path = %directory.paths.aggregate.display(),
                    %error,
                    "startup cgroup orphan cleanup failed"
                );
                pending_cleanup.insert(
                    directory.paths.relative_path.clone(),
                    directory.paths.clone(),
                );
            }
        }
        for trace_id in recovery
            .registered_missing
            .iter()
            .chain(recovery.registered_invalid.iter())
        {
            storage
                .finalize_lost_scope(*trace_id, "registered cgroup scope is missing or invalid")?;
        }
        pending_cleanup.extend(
            recovery
                .unregistered_populated
                .iter()
                .chain(recovery.terminal_populated.iter())
                .map(|directory| {
                    (
                        directory.paths.relative_path.clone(),
                        directory.paths.clone(),
                    )
                }),
        );
        Ok(Self {
            adapter,
            preflight,
            controllers,
            scopes,
            waiting_since,
            finalized_barriers,
            recovered_processes,
            readers: BTreeMap::new(),
            backend_factory,
            pending_admissions: BTreeMap::new(),
            pending_cleanup,
            recovery,
        })
    }

    pub(crate) fn admit_stopped_launch(
        &mut self,
        trace_id: TraceId,
        pid: u32,
        mode: ResourceMetricsMode,
    ) -> Result<CgroupAdmission, ProfilingError> {
        let nonce = Self::random_nonce()?;
        let paths = self
            .adapter
            .trace_paths(trace_id.get(), &nonce)
            .map_err(Self::cgroup_error("cgroup_launch_admission"))?;
        if let Err(error) = self.adapter.create_trace_scope(&paths) {
            return Self::pre_move_failure(mode, error);
        }
        let controller_refs = self
            .controllers
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if let Err(error) = self
            .adapter
            .enable_trace_controllers(&paths, &controller_refs)
        {
            let _ = self.adapter.remove_empty_trace_scope(&paths);
            return Self::pre_move_failure(mode, error);
        }

        // Successful cgroup.procs migration is the admission commit boundary.
        if let Err(error) = self.adapter.move_pid(&paths.workload, pid) {
            self.pending_admissions.insert(trace_id, paths);
            return Err(Self::cgroup_error("cgroup_launch_admission")(error));
        }
        self.pending_admissions.insert(trace_id, paths.clone());
        self.adapter
            .verify_pid_membership(pid, &paths.workload, &self.preflight.hierarchy)
            .map_err(Self::cgroup_error("cgroup_launch_verification"))?;
        Ok(CgroupAdmission::Managed(TraceResourceScope::active(
            trace_id,
            nonce,
            paths.relative_path,
            crate::model::ResourceAccountingMethod::CgroupV2,
            std::time::SystemTime::now(),
        )))
    }

    pub(crate) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ProfilingError> {
        let parsed = self
            .adapter
            .trace_paths_from_relative(&scope.relative_path)
            .map_err(Self::cgroup_error("cgroup_scope_activation"))?;
        if parsed.trace_id != scope.trace_id.get() || parsed.nonce != scope.nonce {
            return Err(ProfilingError::new(
                "cgroup_scope_activation",
                "persisted scope identity does not match its managed path",
            ));
        }
        if let Some(existing) = self.scopes.get(&scope.trace_id) {
            if existing == &parsed.paths {
                return Ok(());
            }
            return Err(ProfilingError::new(
                "cgroup_scope_activation",
                "trace already has a different active resource scope",
            ));
        }
        self.pending_admissions.remove(&scope.trace_id);
        self.recovered_processes.remove(&scope.trace_id);
        self.scopes.insert(scope.trace_id, parsed.paths);
        Ok(())
    }

    pub(crate) fn cleanup_abandoned_admissions(&mut self) {
        let candidates = self
            .pending_admissions
            .iter()
            .map(|(trace_id, paths)| (*trace_id, paths.clone()))
            .collect::<Vec<_>>();
        for (trace_id, paths) in candidates {
            match self.adapter.subtree_is_empty(&paths.aggregate) {
                Ok(true) => match self.adapter.remove_empty_trace_scope(&paths) {
                    Ok(()) => {
                        self.pending_admissions.remove(&trace_id);
                        tracing::info!(%trace_id, "cleaned abandoned cgroup launch admission");
                    }
                    Err(error) => tracing::warn!(
                        %trace_id,
                        %error,
                        "abandoned cgroup launch cleanup failed"
                    ),
                },
                Ok(false) => {}
                Err(error) => tracing::warn!(
                    %trace_id,
                    %error,
                    "abandoned cgroup launch readiness check failed"
                ),
            }
        }
        let cleanup = self
            .pending_cleanup
            .iter()
            .map(|(relative, paths)| (relative.clone(), paths.clone()))
            .collect::<Vec<_>>();
        for (relative, paths) in cleanup {
            match self.adapter.subtree_is_empty(&paths.aggregate) {
                Ok(true) => match self.adapter.remove_empty_trace_scope(&paths) {
                    Ok(()) => {
                        self.pending_cleanup.remove(&relative);
                        tracing::info!(
                            path = %paths.aggregate.display(),
                            "cleaned empty recovered cgroup scope"
                        );
                    }
                    Err(error) => tracing::warn!(
                        path = %paths.aggregate.display(),
                        %error,
                        "recovered cgroup cleanup failed"
                    ),
                },
                Ok(false) => {}
                Err(error) => tracing::warn!(
                    path = %paths.aggregate.display(),
                    %error,
                    "recovered cgroup cleanup readiness check failed"
                ),
            }
        }
    }

    pub(crate) fn scope(&self, id: TraceId) -> Option<&TraceScopePaths> {
        self.scopes.get(&id)
    }
    pub(crate) fn has_scope(&self, id: TraceId) -> bool {
        self.scopes.contains_key(&id)
    }
    pub(crate) fn recovered_processes(
        &self,
    ) -> impl Iterator<Item = (TraceId, ProcessIdentity)> + '_ {
        self.recovered_processes
            .iter()
            .map(|(id, root)| (*id, *root))
    }
    pub(crate) fn take_recovery_state(
        &mut self,
    ) -> (BTreeMap<TraceId, std::time::SystemTime>, BTreeSet<TraceId>) {
        (
            std::mem::take(&mut self.waiting_since),
            std::mem::take(&mut self.finalized_barriers),
        )
    }
    pub(crate) fn populated(
        &self,
        scope: &TraceScopePaths,
    ) -> Result<bool, linux_cgroup::CgroupError> {
        self.adapter.read_populated(&scope.aggregate)
    }
    pub(crate) fn finish(&mut self, id: TraceId, remove_scope: bool) {
        self.recovered_processes.remove(&id);
        self.readers.remove(&id);
        if let Some(paths) = self.scopes.remove(&id) {
            if remove_scope {
                if let Err(error) = self.adapter.remove_empty_trace_scope(&paths) {
                    tracing::warn!(trace_id=%id,%error,"finalized cgroup cleanup failed");
                    self.queue_cleanup(paths);
                }
            } else {
                self.queue_cleanup(paths);
            }
        }
    }

    pub(crate) fn queue_cleanup(&mut self, paths: TraceScopePaths) {
        self.pending_cleanup
            .insert(paths.relative_path.clone(), paths);
    }

    pub(crate) fn sample(
        &mut self,
        trace_id: TraceId,
        root: ProcessIdentity,
        context: SamplingContext,
    ) -> Result<CgroupWorkloadSample, String> {
        if !self.readers.contains_key(&trace_id) {
            let source = self
                .adapter
                .counter_reader(
                    &self
                        .scopes
                        .get(&trace_id)
                        .ok_or("managed scope missing")?
                        .aggregate,
                )
                .map_err(|error| error.to_string())?;
            self.readers
                .insert(trace_id, self.backend_factory.bind(root, source)?);
        }
        self.readers
            .get_mut(&trace_id)
            .expect("backend inserted")
            .sample(&(), context)?
            .ok_or_else(|| "cgroup backend returned no counters".to_string())
    }

    fn random_nonce() -> Result<String, ProfilingError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            ProfilingError::new("cgroup_launch_nonce", format!("generate nonce: {error}"))
        })?;
        let mut nonce = String::with_capacity(bytes.len() * 2);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in bytes {
            nonce.push(char::from(HEX[usize::from(byte >> 4)]));
            nonce.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Ok(nonce)
    }

    fn pre_move_failure(
        mode: ResourceMetricsMode,
        error: linux_cgroup::CgroupError,
    ) -> Result<CgroupAdmission, ProfilingError> {
        let error = Self::cgroup_error("cgroup_launch_admission")(error);
        if mode == ResourceMetricsMode::Auto && error.is_unavailable() {
            Ok(CgroupAdmission::Fallback(error.to_string()))
        } else {
            Err(error)
        }
    }

    pub(super) fn cgroup_error(
        code: &'static str,
    ) -> impl FnOnce(linux_cgroup::CgroupError) -> ProfilingError {
        move |error| {
            let unavailable = matches!(
                code,
                "cgroup_discovery" | "cgroup_preflight" | "cgroup_launch_admission"
            )
                && super::super::cgroup_v2_backend_factory::CgroupAvailability::is_unavailable(
                    &error,
                );
            ProfilingError::new(code, error.to_string()).availability(unavailable)
        }
    }
}

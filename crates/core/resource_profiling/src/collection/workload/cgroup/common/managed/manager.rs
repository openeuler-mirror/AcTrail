// Managed cgroup initialization and startup reconciliation, for any hierarchy.

use std::collections::{BTreeMap, BTreeSet};

use super::super::cgroup_error;
use super::super::sampler::{CgroupSamplerFactory, CgroupScopeSampler, CgroupWorkloadSample};
use super::recovery::ResourceScopeRecovery;
use super::{ManagedHierarchy, ManagedRecoveryRecord, ManagedScopePersistence};
use crate::ProfilingError;
use crate::model::resource_scope::TraceResourceScope;
use crate::{
    ResourceProfilingBackend, SamplingContext, WorkloadBackendMode as ResourceMetricsMode,
};
use linux_cgroup::TraceScopePaths;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

pub(crate) struct ManagedScopeManager<H: ManagedHierarchy> {
    hierarchy: H,
    scopes: BTreeMap<TraceId, TraceScopePaths>,
    waiting_since: BTreeMap<TraceId, std::time::SystemTime>,
    finalized_barriers: BTreeSet<TraceId>,
    recovered_processes: BTreeMap<TraceId, ProcessIdentity>,
    readers: BTreeMap<TraceId, CgroupScopeSampler<H::Reader>>,
    backend_factory: CgroupSamplerFactory,
    pending_admissions: BTreeMap<TraceId, TraceScopePaths>,
    pending_cleanup: BTreeMap<std::path::PathBuf, TraceScopePaths>,
}

pub(crate) enum ManagedScopeAdmission {
    Managed(TraceResourceScope),
    Fallback(String),
}

impl<H: ManagedHierarchy> ManagedScopeManager<H> {
    /// Reconciles persisted scopes with the hierarchy's directories and
    /// cleans up what can be cleaned now.
    pub(crate) fn initialize(
        hierarchy: H,
        recovery_scan_limit: u32,
        backend_factory: CgroupSamplerFactory,
        storage: &mut dyn ManagedScopePersistence,
        records: Vec<ManagedRecoveryRecord>,
    ) -> Result<Self, ProfilingError> {
        let scan_limit = usize::try_from(recovery_scan_limit).map_err(|error| {
            ProfilingError::new("cgroup_recovery", format!("orphan limit overflow: {error}"))
        })?;
        let scan = hierarchy
            .scan_managed_scopes(scan_limit)
            .map_err(cgroup_error("cgroup_recovery"))?;
        let roots = records
            .iter()
            .map(|record| (record.scope.trace_id, record.root))
            .collect::<BTreeMap<_, _>>();
        let registered = records.into_iter().map(|record| record.scope).collect();
        let (scopes, waiting_since, finalized_barriers, recovery) =
            ResourceScopeRecovery::reconcile(&hierarchy, registered, scan)?;
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
            if let Err(error) = hierarchy.remove_empty_trace_scope(&directory.paths) {
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
        tracing::info!(
            accounting_method = H::ACCOUNTING_METHOD.as_str(),
            controllers = ?hierarchy.controllers(),
            recovered_scopes = scopes.len(),
            registered_missing = recovery.registered_missing.len(),
            unregistered_empty = recovery.unregistered_empty.len(),
            unregistered_populated = recovery.unregistered_populated.len(),
            terminal_empty = recovery.terminal_empty.len(),
            terminal_populated = recovery.terminal_populated.len(),
            unknown_directories = recovery.unknown_directories.len(),
            "managed cgroup resource metrics preflight completed"
        );
        Ok(Self {
            hierarchy,
            scopes,
            waiting_since,
            finalized_barriers,
            recovered_processes,
            readers: BTreeMap::new(),
            backend_factory,
            pending_admissions: BTreeMap::new(),
            pending_cleanup,
        })
    }

    pub(crate) fn admit_stopped_launch(
        &mut self,
        trace_id: TraceId,
        pid: u32,
        mode: ResourceMetricsMode,
    ) -> Result<ManagedScopeAdmission, ProfilingError> {
        let nonce = Self::random_nonce()?;
        let paths = self
            .hierarchy
            .trace_paths(trace_id.get(), &nonce)
            .map_err(cgroup_error("cgroup_launch_admission"))?;
        if let Err(error) = self.hierarchy.create_trace_scope(&paths) {
            return Self::pre_move_failure(mode, error);
        }
        if let Err(error) = self.hierarchy.enable_trace_controllers(&paths) {
            let _ = self.hierarchy.remove_empty_trace_scope(&paths);
            return Self::pre_move_failure(mode, error);
        }

        // Successful cgroup.procs migration is the admission commit boundary.
        if let Err(error) = self.hierarchy.move_pid(&paths, pid) {
            self.pending_admissions.insert(trace_id, paths);
            return Err(cgroup_error("cgroup_launch_admission")(error));
        }
        self.pending_admissions.insert(trace_id, paths.clone());
        self.hierarchy
            .verify_pid_membership(pid, &paths)
            .map_err(cgroup_error("cgroup_launch_verification"))?;
        Ok(ManagedScopeAdmission::Managed(TraceResourceScope::active(
            trace_id,
            nonce,
            paths.relative_path,
            H::ACCOUNTING_METHOD,
            std::time::SystemTime::now(),
        )))
    }

    pub(crate) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ProfilingError> {
        if scope.accounting_method != H::ACCOUNTING_METHOD {
            return Err(ProfilingError::new(
                "cgroup_scope_activation",
                "persisted scope accounting method does not match the managed hierarchy",
            ));
        }
        let parsed = self
            .hierarchy
            .trace_paths_from_relative(&scope.relative_path)
            .map_err(cgroup_error("cgroup_scope_activation"))?;
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
            match self.hierarchy.subtree_is_empty(&paths.aggregate) {
                Ok(true) => match self.hierarchy.remove_empty_trace_scope(&paths) {
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
            match self.hierarchy.subtree_is_empty(&paths.aggregate) {
                Ok(true) => match self.hierarchy.remove_empty_trace_scope(&paths) {
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
        self.hierarchy.read_populated(&scope.aggregate)
    }
    pub(crate) fn finish(&mut self, id: TraceId, remove_scope: bool) {
        self.recovered_processes.remove(&id);
        self.readers.remove(&id);
        if let Some(paths) = self.scopes.remove(&id) {
            if remove_scope {
                if let Err(error) = self.hierarchy.remove_empty_trace_scope(&paths) {
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
                .hierarchy
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
    ) -> Result<ManagedScopeAdmission, ProfilingError> {
        let error = cgroup_error("cgroup_launch_admission")(error);
        if mode == ResourceMetricsMode::Auto && error.is_unavailable() {
            Ok(ManagedScopeAdmission::Fallback(error.to_string()))
        } else {
            Err(error)
        }
    }
}

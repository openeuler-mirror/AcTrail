//! Managed cgroup v2 initialization and startup reconciliation.

use std::collections::{BTreeMap, BTreeSet};

use config_core::daemon::{ResourceMetricsConfig, ResourceMetricsMode};
use control_contract::reply::ControlError;
use linux_platform::cgroup_v2::{
    CgroupPreflightReport, ManagedCgroupV2, ManagedScopeDirectory, TraceScopePaths,
    discover_unified_hierarchy,
};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use model_core::resource_scope::{ResourceScopeLifecycleState, TraceResourceScope};
use storage_core::StorageBackend;

pub(super) struct CgroupResourceRuntime {
    pub adapter: ManagedCgroupV2,
    pub preflight: CgroupPreflightReport,
    pub controllers: Vec<String>,
    pub scopes: BTreeMap<TraceId, TraceScopePaths>,
    pub waiting_since: BTreeMap<TraceId, std::time::SystemTime>,
    pub finalized_barriers: BTreeSet<TraceId>,
    pub recovered_processes: BTreeMap<TraceId, ProcessIdentity>,
    pub readers: BTreeMap<TraceId, linux_platform::cgroup_v2::CgroupV2CounterReader>,
    pending_admissions: BTreeMap<TraceId, TraceScopePaths>,
    pending_cleanup: BTreeMap<std::path::PathBuf, TraceScopePaths>,
    pub recovery: ResourceScopeRecovery,
}

pub(super) enum CgroupAdmission {
    Managed(TraceResourceScope),
    Fallback(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ResourceScopeRecovery {
    pub registered_missing: Vec<TraceId>,
    pub registered_invalid: Vec<TraceId>,
    pub unregistered_empty: Vec<ManagedScopeDirectory>,
    pub unregistered_populated: Vec<ManagedScopeDirectory>,
    pub terminal_empty: Vec<ManagedScopeDirectory>,
    pub terminal_populated: Vec<ManagedScopeDirectory>,
    pub unknown_directories: Vec<std::path::PathBuf>,
}

impl CgroupResourceRuntime {
    #[cfg(test)]
    pub(super) fn cleanup_pending(&self, paths: &TraceScopePaths) -> bool {
        self.pending_cleanup.contains_key(&paths.relative_path)
    }

    #[cfg(test)]
    pub(super) fn test_runtime(root: &std::path::Path) -> Self {
        let adapter = ManagedCgroupV2::new(root).unwrap();
        std::fs::create_dir_all(root).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        Self {
            preflight: CgroupPreflightReport {
                hierarchy: linux_platform::cgroup_v2::UnifiedHierarchy {
                    mount_point: root.into(),
                    mount_root: "/".into(),
                    process_relative_path: "/daemon".into(),
                    process_path: root.join("daemon"),
                },
                managed_root: root.into(),
                owner_uid: 0,
                owner_gid: 0,
                available_controllers: BTreeSet::new(),
                enabled_controllers: BTreeSet::new(),
                enabled_optional_controllers: BTreeSet::new(),
                daemon_path: root.join("daemon"),
                disposable_probe_succeeded: true,
                guidance: "test filesystem",
            },
            adapter,
            controllers: Vec::new(),
            scopes: BTreeMap::new(),
            waiting_since: BTreeMap::new(),
            finalized_barriers: BTreeSet::new(),
            recovered_processes: BTreeMap::new(),
            readers: BTreeMap::new(),
            pending_admissions: BTreeMap::new(),
            pending_cleanup: BTreeMap::new(),
            recovery: ResourceScopeRecovery::default(),
        }
    }

    pub(super) fn initialize(
        config: &ResourceMetricsConfig,
        storage: &mut dyn StorageBackend,
    ) -> Result<(Option<Self>, Option<String>), ControlError> {
        if !config.enabled || config.mode == ResourceMetricsMode::Procfs {
            return Ok((None, None));
        }
        // Storage errors are never an availability fallback: without durable
        // state we cannot prove that there is no managed scope to recover.
        let registered = storage
            .list_resource_scopes()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        complete_persisted_finalization_barriers(storage, &registered)?;
        let recovery_required = persisted_scopes_require_recovery(&registered);
        match Self::initialize_required(config, storage, registered) {
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
            Err(error) if config.mode == ResourceMetricsMode::Auto && !recovery_required => {
                let reason = format!("{}: {}", error.code, error.message);
                tracing::warn!(
                    reason = %reason,
                    "cgroup v2 resource metrics unavailable; auto mode will use procfs"
                );
                Ok((None, Some(reason)))
            }
            Err(error) => Err(error),
        }
    }

    fn initialize_required(
        config: &ResourceMetricsConfig,
        storage: &mut dyn StorageBackend,
        registered: Vec<TraceResourceScope>,
    ) -> Result<Self, ControlError> {
        let hierarchy = discover_unified_hierarchy().map_err(cgroup_error("cgroup_discovery"))?;
        let adapter = ManagedCgroupV2::new(config.cgroup_root.clone())
            .map_err(cgroup_error("cgroup_configuration"))?;
        let preflight = adapter
            .prepare_hierarchy(hierarchy, std::process::id())
            .map_err(cgroup_error("cgroup_preflight"))?;
        let controllers = preflight
            .enabled_controllers
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let scan_limit = usize::try_from(config.orphan_limit).map_err(|error| {
            ControlError::new("cgroup_recovery", format!("orphan limit overflow: {error}"))
        })?;
        let scan = adapter
            .scan_managed_scopes(scan_limit)
            .map_err(cgroup_error("cgroup_recovery"))?;
        let (scopes, waiting_since, finalized_barriers, recovery) =
            reconcile_scopes(&adapter, registered, scan)?;
        let mut recovered_processes = BTreeMap::new();
        for trace_id in scopes.keys() {
            let trace = storage
                .get_trace(*trace_id)
                .map_err(|error| ControlError::new(error.stage, error.message))?
                .ok_or_else(|| {
                    ControlError::new(
                        "cgroup_recovery",
                        format!("registered resource scope {trace_id} has no trace record"),
                    )
                })?;
            recovered_processes.insert(*trace_id, trace.root_process_identity);
        }
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
            finalize_lost_scope(
                storage,
                *trace_id,
                "registered cgroup scope is missing or invalid",
            )?;
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
            pending_admissions: BTreeMap::new(),
            pending_cleanup,
            recovery,
        })
    }

    pub(super) fn admit_stopped_launch(
        &mut self,
        trace_id: TraceId,
        pid: u32,
        mode: ResourceMetricsMode,
    ) -> Result<CgroupAdmission, ControlError> {
        let nonce = random_nonce()?;
        let paths = self
            .adapter
            .trace_paths(trace_id.get(), &nonce)
            .map_err(cgroup_error("cgroup_launch_admission"))?;
        if let Err(error) = self.adapter.create_trace_scope(&paths) {
            return pre_move_failure(mode, error);
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
            return pre_move_failure(mode, error);
        }

        // Successful cgroup.procs migration is the admission commit boundary.
        if let Err(error) = self.adapter.move_pid(&paths.workload, pid) {
            self.pending_admissions.insert(trace_id, paths);
            return Err(cgroup_error("cgroup_launch_admission")(error));
        }
        self.pending_admissions.insert(trace_id, paths.clone());
        self.adapter
            .verify_pid_membership(pid, &paths.workload, &self.preflight.hierarchy)
            .map_err(cgroup_error("cgroup_launch_verification"))?;
        Ok(CgroupAdmission::Managed(TraceResourceScope::active(
            trace_id,
            nonce,
            paths.relative_path,
            model_core::event::ResourceAccountingMethod::CgroupV2,
            std::time::SystemTime::now(),
        )))
    }

    pub(super) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ControlError> {
        let parsed = self
            .adapter
            .trace_paths_from_relative(&scope.relative_path)
            .map_err(cgroup_error("cgroup_scope_activation"))?;
        if parsed.trace_id != scope.trace_id.get() || parsed.nonce != scope.nonce {
            return Err(ControlError::new(
                "cgroup_scope_activation",
                "persisted scope identity does not match its managed path",
            ));
        }
        if let Some(existing) = self.scopes.get(&scope.trace_id) {
            if existing == &parsed.paths {
                return Ok(());
            }
            return Err(ControlError::new(
                "cgroup_scope_activation",
                "trace already has a different active resource scope",
            ));
        }
        self.pending_admissions.remove(&scope.trace_id);
        self.recovered_processes.remove(&scope.trace_id);
        self.scopes.insert(scope.trace_id, parsed.paths);
        Ok(())
    }

    pub(super) fn cleanup_abandoned_admissions(&mut self) {
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

    pub(super) fn queue_cleanup(&mut self, paths: TraceScopePaths) {
        self.pending_cleanup
            .insert(paths.relative_path.clone(), paths);
    }

    pub(super) fn read_counters(
        &mut self,
        trace_id: TraceId,
        aggregate: &std::path::Path,
    ) -> Result<linux_platform::cgroup_v2::CgroupCounters, linux_platform::cgroup_v2::CgroupError>
    {
        if !self.readers.contains_key(&trace_id) {
            self.readers
                .insert(trace_id, self.adapter.counter_reader(aggregate)?);
        }
        self.readers
            .get_mut(&trace_id)
            .expect("reader inserted")
            .read_counters()
    }
}

fn complete_persisted_finalization_barriers(
    storage: &mut dyn StorageBackend,
    registered: &[TraceResourceScope],
) -> Result<(), ControlError> {
    // Repair databases written by versions that orphaned a lost scope without
    // persisting any final event or terminal trace transition.
    for scope in registered.iter().filter(|scope| {
        scope.lifecycle_state == ResourceScopeLifecycleState::Orphaned
            && scope.final_event_id.is_none()
    }) {
        finalize_lost_scope(
            storage,
            scope.trace_id,
            "previous recovery orphaned the cgroup without a final event",
        )?;
    }
    for scope in registered
        .iter()
        .filter(|scope| !scope.lifecycle_state.is_live() && scope.final_event_id.is_some())
    {
        let Some(mut trace) = storage
            .get_trace(scope.trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?
        else {
            return Err(ControlError::new(
                "cgroup_recovery",
                format!(
                    "finalized resource scope {} has no trace record",
                    scope.trace_id
                ),
            ));
        };
        if trace.lifecycle_state.is_terminal() {
            continue;
        }
        trace.lifecycle_state = model_core::trace::TraceLifecycleState::Completed;
        trace.timings.completed_at = Some(scope.updated_at);
        if scope.lifecycle_state == ResourceScopeLifecycleState::Orphaned {
            trace.health = model_core::trace::TraceHealth::Degraded;
        }
        storage
            .create_trace(trace)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
    }
    Ok(())
}

fn persisted_scopes_require_recovery(registered: &[TraceResourceScope]) -> bool {
    registered
        .iter()
        .any(|scope| scope.lifecycle_state.is_live())
}

fn finalize_lost_scope(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    reason: &str,
) -> Result<(), ControlError> {
    use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
    use model_core::event::{
        DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload,
        ResourceAccountingCoverage, ResourceAccountingMethod, ResourcePayload, ResourceSampleKind,
    };
    use model_core::ids::{CollectorName, DiagnosticId, EventId};
    use model_core::trace::{TraceHealth, TraceLifecycleState};
    let error = |error: storage_core::StorageError| ControlError::new(error.stage, error.message);
    let scope = storage
        .get_resource_scope(trace_id)
        .map_err(error)?
        .ok_or_else(|| ControlError::new("cgroup_recovery", "resource scope record is missing"))?;
    if scope.final_event_id.is_some() {
        return Ok(());
    }
    let mut trace = storage.get_trace(trace_id).map_err(error)?.ok_or_else(|| {
        ControlError::new("cgroup_recovery", "resource scope has no trace record")
    })?;
    let now = std::time::SystemTime::now();
    let event_id = EventId::new(storage.next_event_id_seed().map_err(error)?);
    let diagnostic_id = DiagnosticId::new(storage.next_diagnostic_id_seed().map_err(error)?);
    let event = DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id,
            observed_at: now,
            process: trace.root_process_identity,
            collector: CollectorName::new(super::COLLECTOR_NAME),
            kind: EventKind::Resource,
            flags: EventFlags {
                metadata_partial: true,
                ..EventFlags::clean()
            },
        },
        EventPayload::Resource(ResourcePayload {
            scope: "trace".to_string(),
            subject: trace_id.to_string(),
            accounting_method: ResourceAccountingMethod::CgroupV2,
            accounting_coverage: ResourceAccountingCoverage::Partial,
            sample_kind: ResourceSampleKind::Final,
            metadata: BTreeMap::from([("recovery_scope_error".to_string(), reason.to_string())]),
            ..ResourcePayload::default()
        }),
    );
    if !trace.lifecycle_state.is_terminal() {
        trace.lifecycle_state = TraceLifecycleState::Completed;
        trace.timings.completed_at = Some(now);
    }
    trace.health = TraceHealth::Degraded;
    let diagnostic = DiagnosticRecord::new(
        diagnostic_id,
        Some(trace_id),
        DiagnosticKind::RuntimeFailure,
        DiagnosticSeverity::Warning,
        now,
        reason,
    )
    .with_metadata("resource_scope_recovery", "lost");
    let transaction = storage.begin().map_err(error)?;
    let result = (|| {
        storage.append_event(event)?;
        storage.update_resource_scope_state(
            trace_id,
            ResourceScopeLifecycleState::Orphaned,
            Some(event_id),
            now,
        )?;
        storage.create_trace(trace)?;
        storage.append_diagnostic(diagnostic)
    })();
    match result {
        Ok(()) => transaction.commit().map_err(error),
        Err(failure) => {
            transaction.rollback().map_err(error)?;
            Err(error(failure))
        }
    }
}

fn random_nonce() -> Result<String, ControlError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ControlError::new("cgroup_launch_nonce", format!("generate nonce: {error}"))
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
    error: linux_platform::cgroup_v2::CgroupError,
) -> Result<CgroupAdmission, ControlError> {
    if mode == ResourceMetricsMode::Auto {
        Ok(CgroupAdmission::Fallback(error.to_string()))
    } else {
        Err(ControlError::new(
            "cgroup_launch_admission",
            error.to_string(),
        ))
    }
}

fn reconcile_scopes(
    adapter: &ManagedCgroupV2,
    registered: Vec<TraceResourceScope>,
    scan: linux_platform::cgroup_v2::ManagedScopeScan,
) -> Result<
    (
        BTreeMap<TraceId, TraceScopePaths>,
        BTreeMap<TraceId, std::time::SystemTime>,
        BTreeSet<TraceId>,
        ResourceScopeRecovery,
    ),
    ControlError,
> {
    let mut recovery = ResourceScopeRecovery {
        unknown_directories: scan.unknown,
        ..ResourceScopeRecovery::default()
    };
    let mut directories = BTreeMap::<u64, Vec<ManagedScopeDirectory>>::new();
    for directory in scan.managed {
        directories
            .entry(directory.trace_id)
            .or_default()
            .push(directory);
    }
    let mut scopes = BTreeMap::new();
    let mut waiting_since = BTreeMap::new();
    let mut finalized_barriers = BTreeSet::new();

    for record in registered {
        if !record.lifecycle_state.is_live() && record.final_event_id.is_some() {
            finalized_barriers.insert(record.trace_id);
        }
        let parsed = match adapter.trace_paths_from_relative(&record.relative_path) {
            Ok(parsed)
                if parsed.trace_id == record.trace_id.get() && parsed.nonce == record.nonce =>
            {
                parsed
            }
            _ => {
                if record.lifecycle_state.is_live() {
                    recovery.registered_invalid.push(record.trace_id);
                }
                continue;
            }
        };
        let Some(candidates) = directories.get_mut(&parsed.trace_id) else {
            if record.lifecycle_state.is_live() {
                recovery.registered_missing.push(record.trace_id);
            }
            continue;
        };
        let Some(index) = candidates.iter().position(|directory| {
            directory.paths == parsed.paths && directory.nonce == parsed.nonce
        }) else {
            if record.lifecycle_state.is_live() {
                recovery.registered_invalid.push(record.trace_id);
            }
            continue;
        };
        let directory = candidates.remove(index);
        if record.lifecycle_state.is_live() {
            if record.lifecycle_state == ResourceScopeLifecycleState::WaitingForEmpty {
                waiting_since.insert(record.trace_id, record.updated_at);
            }
            scopes.insert(record.trace_id, directory.paths);
        } else {
            let empty = adapter
                .subtree_is_empty(&directory.paths.aggregate)
                .map_err(cgroup_error("cgroup_recovery"))?;
            if empty {
                recovery.terminal_empty.push(directory);
            } else {
                recovery.terminal_populated.push(directory);
            }
        }
    }

    for candidates in directories.into_values() {
        for directory in candidates {
            let empty = adapter
                .subtree_is_empty(&directory.paths.aggregate)
                .map_err(cgroup_error("cgroup_recovery"))?;
            if empty {
                recovery.unregistered_empty.push(directory);
            } else {
                recovery.unregistered_populated.push(directory);
            }
        }
    }
    recovery.registered_missing.sort();
    recovery.registered_invalid.sort();
    Ok((scopes, waiting_since, finalized_barriers, recovery))
}

fn cgroup_error(
    code: &'static str,
) -> impl FnOnce(linux_platform::cgroup_v2::CgroupError) -> ControlError {
    move |error| ControlError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::UNIX_EPOCH;

    use linux_platform::cgroup_v2::ManagedCgroupV2;
    use model_core::event::ResourceAccountingMethod;

    use super::*;

    #[test]
    fn reconciliation_matches_registry_and_classifies_orphans_without_deleting() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("actrail");
        fs::create_dir(&root).unwrap();
        let adapter = ManagedCgroupV2::new(&root).unwrap();
        adapter.create_managed_hierarchy().unwrap();

        let registered_paths = adapter.trace_paths(1, "registered").unwrap();
        adapter.create_trace_scope(&registered_paths).unwrap();
        let empty_paths = adapter.trace_paths(2, "empty").unwrap();
        adapter.create_trace_scope(&empty_paths).unwrap();
        write_procs(&empty_paths, "");
        let populated_paths = adapter.trace_paths(3, "populated").unwrap();
        adapter.create_trace_scope(&populated_paths).unwrap();
        write_procs(&populated_paths, "44\n");
        fs::create_dir(root.join("traces/foreign")).unwrap();

        let record = TraceResourceScope::active(
            TraceId::new(1),
            "registered",
            registered_paths.relative_path.clone(),
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        let scan = adapter.scan_managed_scopes(8).unwrap();
        let (scopes, waiting_since, finalized_barriers, recovery) =
            reconcile_scopes(&adapter, vec![record], scan).unwrap();

        assert_eq!(scopes.get(&TraceId::new(1)), Some(&registered_paths));
        assert!(waiting_since.is_empty());
        assert!(finalized_barriers.is_empty());
        assert_eq!(recovery.unregistered_empty.len(), 1);
        assert_eq!(recovery.unregistered_empty[0].trace_id, 2);
        assert_eq!(recovery.unregistered_populated.len(), 1);
        assert_eq!(recovery.unregistered_populated[0].trace_id, 3);
        assert_eq!(
            recovery.unknown_directories,
            vec![root.join("traces/foreign")]
        );
        assert!(empty_paths.aggregate.exists());
        assert!(populated_paths.aggregate.exists());
    }

    #[test]
    fn reconciliation_reports_missing_and_invalid_live_records() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("actrail");
        fs::create_dir(&root).unwrap();
        let adapter = ManagedCgroupV2::new(&root).unwrap();
        adapter.create_managed_hierarchy().unwrap();
        let missing = TraceResourceScope::active(
            TraceId::new(4),
            "missing",
            "traces/trace-4-missing",
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        let invalid = TraceResourceScope::active(
            TraceId::new(5),
            "expected",
            "../escape",
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        let scan = adapter.scan_managed_scopes(8).unwrap();
        let (_, _, _, recovery) = reconcile_scopes(&adapter, vec![missing, invalid], scan).unwrap();
        assert_eq!(recovery.registered_missing, vec![TraceId::new(4)]);
        assert_eq!(recovery.registered_invalid, vec![TraceId::new(5)]);
    }

    #[test]
    fn reconciliation_resumes_waiting_deadline_and_does_not_reactivate_finalized_scope() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("actrail");
        fs::create_dir(&root).unwrap();
        let adapter = ManagedCgroupV2::new(&root).unwrap();
        adapter.create_managed_hierarchy().unwrap();

        let waiting_paths = adapter.trace_paths(6, "waiting").unwrap();
        adapter.create_trace_scope(&waiting_paths).unwrap();
        write_procs(&waiting_paths, "61\n");
        let finalized_paths = adapter.trace_paths(7, "finalized").unwrap();
        adapter.create_trace_scope(&finalized_paths).unwrap();
        write_procs(&finalized_paths, "");

        let mut waiting = TraceResourceScope::active(
            TraceId::new(6),
            "waiting",
            waiting_paths.relative_path.clone(),
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        waiting.lifecycle_state = ResourceScopeLifecycleState::WaitingForEmpty;
        waiting.updated_at = UNIX_EPOCH + std::time::Duration::from_secs(20);
        let mut finalized = TraceResourceScope::active(
            TraceId::new(7),
            "finalized",
            finalized_paths.relative_path.clone(),
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        finalized.lifecycle_state = ResourceScopeLifecycleState::Finalized;
        finalized.final_event_id = Some(model_core::ids::EventId::new(70));

        let scan = adapter.scan_managed_scopes(8).unwrap();
        let (scopes, waiting_since, finalized_barriers, recovery) =
            reconcile_scopes(&adapter, vec![waiting.clone(), finalized], scan).unwrap();

        assert_eq!(scopes.get(&waiting.trace_id), Some(&waiting_paths));
        assert_eq!(
            waiting_since.get(&waiting.trace_id),
            Some(&waiting.updated_at)
        );
        assert_eq!(finalized_barriers, BTreeSet::from([TraceId::new(7)]));
        assert_eq!(recovery.terminal_empty.len(), 1);
        assert_eq!(recovery.terminal_empty[0].trace_id, 7);
        assert!(!scopes.contains_key(&TraceId::new(7)));
    }

    #[test]
    fn auto_fallback_is_blocked_by_a_durable_live_scope() {
        let active = TraceResourceScope::active(
            TraceId::new(8),
            "active",
            "traces/trace-8-active",
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        let mut finalized = TraceResourceScope::active(
            TraceId::new(9),
            "finalized",
            "traces/trace-9-finalized",
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH,
        );
        finalized.lifecycle_state = ResourceScopeLifecycleState::Finalized;
        finalized.final_event_id = Some(model_core::ids::EventId::new(90));

        assert!(persisted_scopes_require_recovery(&[active]));
        assert!(!persisted_scopes_require_recovery(&[finalized]));
    }

    fn write_procs(paths: &TraceScopePaths, workload: &str) {
        fs::write(paths.aggregate.join("cgroup.procs"), "").unwrap();
        fs::write(paths.workload.join("cgroup.procs"), workload).unwrap();
    }

    #[test]
    fn lost_scope_recovery_persists_terminal_trace_final_event_and_diagnostic_once() {
        use model_core::ids::{OtelTraceId, ProfileName, TraceName};
        use model_core::trace::{TraceAlertToken, TraceHealth, TraceLifecycleState, TraceRecord};
        let temp = tempfile::tempdir().unwrap();
        let config =
            storage_factory::StorageConfig::sqlite_path(temp.path().join("recovery.sqlite"));
        let mut storage = storage_factory::open_storage_backend(
            &config,
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        for (id, state) in [
            (1, ResourceScopeLifecycleState::Active),
            (2, ResourceScopeLifecycleState::Orphaned),
        ] {
            let trace_id = TraceId::new(id);
            let trace = TraceRecord::new(
                trace_id,
                OtelTraceId::from_bytes([id as u8; 16]).unwrap(),
                TraceAlertToken::new([0; 32]),
                ProcessIdentity::new(id),
                TraceName::new("recovery"),
                ProfileName::new("test"),
                UNIX_EPOCH,
            );
            storage.create_trace(trace).unwrap();
            let mut scope = TraceResourceScope::active(
                trace_id,
                "lost",
                format!("traces/trace-{id}-lost"),
                ResourceAccountingMethod::CgroupV2,
                UNIX_EPOCH,
            );
            scope.lifecycle_state = state;
            storage.create_resource_scope(scope).unwrap();
        }
        finalize_lost_scope(storage.as_mut(), TraceId::new(1), "missing after reboot").unwrap();
        let registered = storage.list_resource_scopes().unwrap();
        complete_persisted_finalization_barriers(storage.as_mut(), &registered).unwrap();
        drop(storage);
        let mut storage = storage_factory::open_storage_backend(
            &config,
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        for id in [1, 2] {
            let trace_id = TraceId::new(id);
            finalize_lost_scope(storage.as_mut(), trace_id, "second recovery").unwrap();
            let trace = storage.get_trace(trace_id).unwrap().unwrap();
            assert_eq!(trace.lifecycle_state, TraceLifecycleState::Completed);
            assert_eq!(trace.health, TraceHealth::Degraded);
            assert!(trace.timings.completed_at.is_some());
            assert!(
                storage
                    .get_resource_scope(trace_id)
                    .unwrap()
                    .unwrap()
                    .final_event_id
                    .is_some()
            );
            let events = storage.list_events(trace_id).unwrap();
            assert_eq!(events.len(), 1);
            let model_core::event::EventPayload::Resource(payload) = &events[0].payload else {
                panic!("not resource");
            };
            assert_eq!(
                payload.accounting_coverage,
                model_core::event::ResourceAccountingCoverage::Partial
            );
            assert_eq!(payload.memory_current_bytes, None);
            assert!(payload.metadata.contains_key("recovery_scope_error"));
            assert_eq!(storage.list_diagnostics(trace_id).unwrap().len(), 1);
        }
        assert_eq!(storage.next_event_id_seed().unwrap(), 3);
        assert_eq!(storage.next_diagnostic_id_seed().unwrap(), 3);
    }

    #[test]
    fn cleanup_retries_and_forgets_externally_removed_scope() {
        let temp = tempfile::tempdir().unwrap();
        let mut runtime = CgroupResourceRuntime::test_runtime(&temp.path().join("cg"));
        let paths = runtime.adapter.trace_paths(1, "retry").unwrap();
        runtime.adapter.create_trace_scope(&paths).unwrap();
        write_procs(&paths, "");
        runtime.queue_cleanup(paths.clone());
        // Regular fixture files simulate a removal failure; they are virtual
        // controller files on a real cgroup filesystem.
        runtime.cleanup_abandoned_admissions();
        assert!(runtime.pending_cleanup.contains_key(&paths.relative_path));
        std::fs::remove_dir_all(&paths.aggregate).unwrap();
        runtime.cleanup_abandoned_admissions();
        assert!(runtime.pending_cleanup.is_empty());
    }
}

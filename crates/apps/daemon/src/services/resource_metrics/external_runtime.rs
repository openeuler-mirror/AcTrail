//! Read-only host container cgroup runtime for attached container traces.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::SystemTime;

use config_core::daemon::{ExistingContainerCgroups, ResourceMetricsConfig};
use control_contract::reply::ControlError;
use linux_platform::cgroup_v2::{CgroupCounters, UnifiedHierarchy, discover_unified_hierarchy};
use linux_platform::container_cgroup::{
    ExternalCgroupV2, bind_host_container_cgroup, reopen_host_container_cgroup,
};
use model_core::external_cgroup::{
    ExternalBindingStaleReason, ExternalBindingState, ExternalCgroupBinding, HostBootId,
};
use model_core::ids::TraceId;
use model_core::process::{HostProcessCoordinates, ProcessIdentity};
use storage_core::StorageBackend;

use super::external::{read_host_boot_id, stale_reason_for_open_error};

/// One read-only handle to a runtime-owned host container cgroup.
pub(super) struct ExternalContainerSource {
    pub binding: ExternalCgroupBinding,
    pub reader: ExternalCgroupV2,
}

pub(super) struct ExternalCgroupRuntime {
    procfs_root: std::path::PathBuf,
    hierarchy: Option<UnifiedHierarchy>,
    boot_id: HostBootId,
    sources: BTreeMap<TraceId, ExternalContainerSource>,
    stale: BTreeSet<TraceId>,
    recovered_processes: BTreeMap<TraceId, ProcessIdentity>,
    recovered_terminal: BTreeSet<TraceId>,
}

pub(super) enum ExternalAdmission {
    Bound,
    Fallback(String),
}

pub(super) enum ExternalRead {
    Counters(CgroupCounters),
    BecameStale(ExternalBindingStaleReason),
    Retryable(String),
}

pub(super) struct ExternalFinalDraft {
    pub binding: ExternalCgroupBinding,
    pub counters: Option<CgroupCounters>,
    pub read_error: Option<String>,
}

impl ExternalCgroupRuntime {
    pub(super) fn initialize(
        config: &ResourceMetricsConfig,
        storage: &mut dyn StorageBackend,
    ) -> Result<Option<Self>, ControlError> {
        if !config.enabled
            || config.existing_container_cgroups == ExistingContainerCgroups::Disabled
        {
            return Ok(None);
        }
        let boot_id = read_host_boot_id(Path::new("/proc")).map_err(|error| {
            ControlError::new(
                "external_cgroup_recovery",
                format!("cannot read host boot identity: {error}"),
            )
        })?;
        let hierarchy = match discover_unified_hierarchy() {
            Ok(hierarchy) => Some(hierarchy),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "cgroup2 discovery failed; external container accounting unavailable"
                );
                None
            }
        };
        let mut bindings = storage
            .list_live_external_cgroup_bindings()
            .map_err(storage_error)?;
        let mut recovered_processes = BTreeMap::new();
        let mut recovered_terminal = BTreeSet::new();
        for binding in &bindings {
            let Some(trace) = storage.get_trace(binding.trace_id).map_err(storage_error)? else {
                use model_core::diagnostics::{
                    DiagnosticKind, DiagnosticRecord, DiagnosticSeverity,
                };
                let diagnostic = DiagnosticRecord::new(
                    model_core::ids::DiagnosticId::new(
                        storage.next_diagnostic_id_seed().map_err(storage_error)?,
                    ),
                    None,
                    DiagnosticKind::RuntimeFailure,
                    DiagnosticSeverity::Warning,
                    SystemTime::now(),
                    format!(
                        "discarded interrupted external admission for missing trace {}",
                        binding.trace_id
                    ),
                );
                let transaction = storage.begin().map_err(storage_error)?;
                let result = storage
                    .append_diagnostic(diagnostic)
                    .and_then(|_| storage.discard_orphan_external_binding(binding.trace_id));
                if let Err(error) = result {
                    transaction.rollback().map_err(storage_error)?;
                    return Err(storage_error(error));
                }
                transaction.commit().map_err(storage_error)?;
                tracing::warn!(trace_id = %binding.trace_id, "discarded interrupted external admission without trace");
                continue;
            };
            recovered_processes.insert(binding.trace_id, trace.root_process_identity);
            if trace.lifecycle_state.is_terminal() {
                recovered_terminal.insert(binding.trace_id);
            }
        }
        bindings.retain(|binding| recovered_processes.contains_key(&binding.trace_id));
        let mut runtime = Self {
            procfs_root: "/proc".into(),
            hierarchy,
            boot_id,
            sources: BTreeMap::new(),
            stale: BTreeSet::new(),
            recovered_processes,
            recovered_terminal,
        };
        runtime.reconcile(storage, &bindings)?;
        Ok(Some(runtime))
    }

    fn reconcile(
        &mut self,
        storage: &mut dyn StorageBackend,
        bindings: &[ExternalCgroupBinding],
    ) -> Result<(), ControlError> {
        let now = SystemTime::now();
        for binding in bindings {
            if binding.lifecycle_state == ExternalBindingState::Stale {
                persist_recovery_stale(
                    storage,
                    binding.trace_id,
                    binding
                        .stale_reason
                        .unwrap_or(ExternalBindingStaleReason::BoundaryMissing),
                    now,
                )?;
                self.stale.insert(binding.trace_id);
                continue;
            }
            let reason = if binding.host_boot_id != self.boot_id {
                Some(ExternalBindingStaleReason::HostBootChanged)
            } else if self.hierarchy.is_none() {
                Some(ExternalBindingStaleReason::BoundaryMissing)
            } else {
                self.reopen(binding).err()
            };
            if let Some(reason) = reason {
                persist_recovery_stale(storage, binding.trace_id, reason, now)?;
                self.stale.insert(binding.trace_id);
            }
        }
        Ok(())
    }

    fn reopen(
        &mut self,
        binding: &ExternalCgroupBinding,
    ) -> Result<(), ExternalBindingStaleReason> {
        let hierarchy = self
            .hierarchy
            .as_ref()
            .ok_or(ExternalBindingStaleReason::BoundaryMissing)?;
        let reopened = reopen_host_container_cgroup(
            hierarchy,
            binding.runtime,
            binding.container_id,
            &binding.relative_path,
        )
        .map_err(stale_reason_for_open_error)?;
        if reopened.directory_identity.device != binding.cgroup_device
            || reopened.directory_identity.inode != binding.cgroup_inode
        {
            return Err(ExternalBindingStaleReason::BoundaryIdentityChanged);
        }
        self.sources.insert(
            binding.trace_id,
            ExternalContainerSource {
                binding: binding.clone(),
                reader: reopened,
            },
        );
        Ok(())
    }

    pub(super) fn admit(
        &mut self,
        trace_id: TraceId,
        coordinates: &HostProcessCoordinates,
        require: bool,
    ) -> Result<ExternalAdmission, ControlError> {
        if self.sources.contains_key(&trace_id) || self.stale.contains(&trace_id) {
            return Ok(ExternalAdmission::Fallback(
                "trace already has an external cgroup binding".to_string(),
            ));
        }
        let Some(hierarchy) = self.hierarchy.as_ref() else {
            let reason = "cgroup v2 unified hierarchy unavailable".to_string();
            if require {
                return Err(ControlError::new("external_cgroup_admission", reason));
            }
            return Ok(ExternalAdmission::Fallback(reason));
        };
        match bind_host_container_cgroup(&self.procfs_root, hierarchy, coordinates) {
            Ok(mut reader) => {
                if let Err(error) = reader.read_counters() {
                    let reason = format!("required external counters unavailable: {error}");
                    if require {
                        return Err(ControlError::new("external_cgroup_admission", reason));
                    }
                    return Ok(ExternalAdmission::Fallback(reason));
                }
                let binding = ExternalCgroupBinding::active(
                    trace_id,
                    reader.identity.runtime,
                    reader.identity.container_id,
                    reader.relative_path.clone(),
                    reader.directory_identity.device,
                    reader.directory_identity.inode,
                    self.boot_id,
                    SystemTime::now(),
                );
                // Prepared only: finalize_trace commits this with the trace and processes.
                self.sources.insert(
                    trace_id,
                    ExternalContainerSource {
                        binding: binding.clone(),
                        reader,
                    },
                );
                Ok(ExternalAdmission::Bound)
            }
            Err(error) => {
                let reason = format!("external container cgroup bind failed: {error}");
                if require {
                    return Err(ControlError::new("external_cgroup_admission", reason));
                }
                Ok(ExternalAdmission::Fallback(reason))
            }
        }
    }

    pub(super) fn read(
        &mut self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
        coordinates: Option<&HostProcessCoordinates>,
        observed_at: SystemTime,
        failure_threshold: u32,
    ) -> Result<ExternalRead, ControlError> {
        let Some(source) = self.sources.get_mut(&trace_id) else {
            return Ok(ExternalRead::Retryable(
                "trace has no active external cgroup source".to_string(),
            ));
        };
        let verification = coordinates
            .ok_or(ExternalBindingStaleReason::ContainerIdentityMismatch)
            .and_then(|host| {
                source
                    .reader
                    .verify_membership(&self.procfs_root, host)
                    .map_err(stale_reason_for_open_error)
            });
        if let Err(reason) = verification {
            storage
                .mark_external_cgroup_stale(trace_id, reason, observed_at)
                .map_err(storage_error)?;
            self.sources.remove(&trace_id);
            self.stale.insert(trace_id);
            return Ok(ExternalRead::BecameStale(reason));
        }
        match source.reader.read_counters() {
            Ok(counters) => {
                storage
                    .record_external_cgroup_success(trace_id, observed_at)
                    .map_err(storage_error)?;
                Ok(ExternalRead::Counters(counters))
            }
            Err(error) => {
                let failures = storage
                    .record_external_cgroup_failure(trace_id, observed_at)
                    .map_err(storage_error)?;
                if failures >= failure_threshold {
                    storage
                        .mark_external_cgroup_stale(
                            trace_id,
                            ExternalBindingStaleReason::RepeatedReadFailure,
                            observed_at,
                        )
                        .map_err(storage_error)?;
                    self.sources.remove(&trace_id);
                    self.stale.insert(trace_id);
                    Ok(ExternalRead::BecameStale(
                        ExternalBindingStaleReason::RepeatedReadFailure,
                    ))
                } else {
                    Ok(ExternalRead::Retryable(error.to_string()))
                }
            }
        }
    }

    pub(super) fn prepare_final(
        &mut self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
    ) -> Result<Option<ExternalFinalDraft>, ControlError> {
        // A collector admission can fail after preparing a source but before the
        // atomic trace write. It has no durable finalization obligation.
        if storage
            .get_external_cgroup_binding(trace_id)
            .map_err(storage_error)?
            .is_none()
        {
            self.remove(trace_id);
            return Ok(None);
        }
        if let Some(source) = self.sources.get_mut(&trace_id) {
            let (counters, read_error) = match source.reader.read_counters() {
                Ok(counters) => (Some(counters), None),
                Err(error) => (None, Some(error.to_string())),
            };
            return Ok(Some(ExternalFinalDraft {
                binding: source.binding.clone(),
                counters,
                read_error,
            }));
        }
        if self.stale.contains(&trace_id) {
            let binding = storage
                .get_external_cgroup_binding(trace_id)
                .map_err(storage_error)?
                .ok_or_else(|| {
                    ControlError::new(
                        "external_cgroup_finalization",
                        "stale binding record is missing from storage",
                    )
                })?;
            return Ok(Some(ExternalFinalDraft {
                binding,
                counters: None,
                read_error: None,
            }));
        }
        Ok(None)
    }

    pub(super) fn remove(&mut self, trace_id: TraceId) {
        self.sources.remove(&trace_id);
        self.stale.remove(&trace_id);
        self.recovered_processes.remove(&trace_id);
        self.recovered_terminal.remove(&trace_id);
    }

    pub(super) fn binding(&self, trace_id: TraceId) -> Option<&ExternalCgroupBinding> {
        self.sources.get(&trace_id).map(|source| &source.binding)
    }

    pub(super) fn boundary_path(&self, trace_id: TraceId) -> Option<std::path::PathBuf> {
        Some(
            self.hierarchy
                .as_ref()?
                .mount_point
                .join(&self.binding(trace_id)?.relative_path),
        )
    }

    pub(super) fn is_active(&self, trace_id: TraceId) -> bool {
        self.sources.contains_key(&trace_id)
    }

    pub(super) fn is_stale(&self, trace_id: TraceId) -> bool {
        self.stale.contains(&trace_id)
    }

    pub(super) fn has_pending(&self, trace_id: TraceId) -> bool {
        self.is_active(trace_id) || self.is_stale(trace_id)
    }

    pub(super) fn recovered_processes(
        &self,
    ) -> impl Iterator<Item = (TraceId, ProcessIdentity)> + '_ {
        self.recovered_processes
            .iter()
            .map(|(trace_id, process)| (*trace_id, *process))
    }

    pub(super) fn recovered_terminal_processes(
        &self,
    ) -> impl Iterator<Item = (TraceId, ProcessIdentity)> + '_ {
        self.recovered_terminal.iter().filter_map(|trace_id| {
            self.recovered_processes
                .get(trace_id)
                .map(|process| (*trace_id, *process))
        })
    }
}

fn storage_error(error: storage_core::StorageError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn persist_recovery_stale(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    reason: ExternalBindingStaleReason,
    now: SystemTime,
) -> Result<(), ControlError> {
    use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
    use model_core::trace::TraceHealth;
    let already_diagnosed = storage
        .list_diagnostics(trace_id)
        .map_err(storage_error)?
        .iter()
        .any(|record| record.metadata.contains_key("external_cgroup_stale"));
    let diagnostic_id = storage.next_diagnostic_id_seed().map_err(storage_error)?;
    let transaction = storage.begin().map_err(storage_error)?;
    let result = (|| {
        storage.mark_external_cgroup_stale(trace_id, reason, now)?;
        storage.update_trace_health(trace_id, TraceHealth::Degraded)?;
        if !already_diagnosed {
            storage.append_diagnostic(
                DiagnosticRecord::new(
                    model_core::ids::DiagnosticId::new(diagnostic_id),
                    Some(trace_id),
                    DiagnosticKind::RuntimeFailure,
                    DiagnosticSeverity::Warning,
                    now,
                    "external cgroup accounting lost during recovery; using procfs fallback",
                )
                .with_metadata("external_cgroup_stale", reason.as_storage_str()),
            )?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => transaction.commit().map_err(storage_error),
        Err(error) => {
            transaction.rollback().map_err(storage_error)?;
            Err(storage_error(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model_core::container::{ContainerRuntime, NormalizedContainerId};
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn runtime(hierarchy: UnifiedHierarchy) -> ExternalCgroupRuntime {
        ExternalCgroupRuntime {
            procfs_root: "/proc".into(),
            hierarchy: Some(hierarchy),
            boot_id: HostBootId::from_bytes([1; 16]),
            sources: BTreeMap::new(),
            stale: BTreeSet::new(),
            recovered_processes: BTreeMap::new(),
            recovered_terminal: BTreeSet::new(),
        }
    }

    #[test]
    fn admission_checks_required_counters_and_sampling_checks_membership() {
        let temp = tempfile::tempdir().unwrap();
        let procfs = temp.path().join("proc");
        let mount = temp.path().join("cg");
        let boundary = mount.join(format!("docker/{ID}"));
        fs::create_dir_all(&boundary).unwrap();
        fs::create_dir_all(procfs.join("42")).unwrap();
        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        fields[19] = "900";
        fs::write(
            procfs.join("42/stat"),
            format!("42 (test) {}", fields.join(" ")),
        )
        .unwrap();
        fs::write(procfs.join("42/cgroup"), format!("0::/docker/{ID}\n")).unwrap();
        let mut runtime = runtime(UnifiedHierarchy {
            mount_point: mount.clone(),
            mount_root: "/".into(),
            process_relative_path: "/".into(),
            process_path: mount,
        });
        runtime.procfs_root = procfs.clone();
        let mut storage = storage_factory::open_storage_backend(
            &storage_factory::StorageConfig::sqlite_path(temp.path().join("test.sqlite")),
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        let id = TraceId::new(1);
        let host = HostProcessCoordinates::new(42, 900);
        assert!(runtime.admit(id, &host, true).is_err());
        assert!(matches!(
            runtime.admit(id, &host, false).unwrap(),
            ExternalAdmission::Fallback(_)
        ));
        fs::write(boundary.join("memory.current"), "1024\n").unwrap();
        assert!(runtime.admit(id, &host, true).is_err());
        fs::write(boundary.join("cpu.stat"), "usage_usec 100\n").unwrap();
        assert!(matches!(
            runtime.admit(id, &host, true).unwrap(),
            ExternalAdmission::Bound
        ));
        // Admission is only prepared: no crash-visible binding precedes the trace.
        assert!(storage.get_external_cgroup_binding(id).unwrap().is_none());
        storage
            .create_external_cgroup_binding(runtime.binding(id).unwrap().clone())
            .unwrap();
        assert!(matches!(
            runtime
                .read(storage.as_mut(), id, Some(&host), SystemTime::now(), 3)
                .unwrap(),
            ExternalRead::Counters(_)
        ));
        fs::write(procfs.join("42/cgroup"), "0::/outside\n").unwrap();
        assert!(matches!(
            runtime
                .read(storage.as_mut(), id, Some(&host), SystemTime::now(), 3)
                .unwrap(),
            ExternalRead::BecameStale(_)
        ));
        assert!(!runtime.is_active(id));
        assert_eq!(
            storage
                .get_external_cgroup_binding(id)
                .unwrap()
                .unwrap()
                .lifecycle_state,
            ExternalBindingState::Stale
        );
    }

    #[test]
    fn recovered_active_trace_waits_for_identity_exit_then_becomes_terminal() {
        use super::super::{ProcessIdentityManager, ResourceMetricsSampler};
        use model_core::ids::{OtelTraceId, ProfileName, TraceName};
        use model_core::process::{ProcessObservation, ProcessRecord};
        use model_core::trace::{TraceAlertToken, TraceLifecycleState, TraceRecord};
        let temp = tempfile::tempdir().unwrap();
        let mut storage = storage_factory::open_storage_backend(
            &storage_factory::StorageConfig::sqlite_path(temp.path().join("test.sqlite")),
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        let id = TraceId::new(1);
        let process = ProcessIdentity::new(1);
        let mut trace = TraceRecord::new(
            id,
            OtelTraceId::from_bytes([1; 16]).unwrap(),
            TraceAlertToken::new([0; 32]),
            process,
            TraceName::new("recovered"),
            ProfileName::new("test"),
            UNIX_EPOCH,
        );
        trace.lifecycle_state = TraceLifecycleState::Active;
        storage.create_trace(trace).unwrap();
        storage
            .create_external_cgroup_binding(ExternalCgroupBinding::active(
                id,
                ContainerRuntime::Docker,
                NormalizedContainerId::from_lower_hex(ID).unwrap(),
                format!("docker/{ID}"),
                1,
                2,
                HostBootId::from_bytes([1; 16]),
                UNIX_EPOCH,
            ))
            .unwrap();
        storage
            .mark_external_cgroup_stale(
                id,
                ExternalBindingStaleReason::RepeatedReadFailure,
                UNIX_EPOCH,
            )
            .unwrap();
        let mut external = runtime(UnifiedHierarchy {
            mount_point: temp.path().into(),
            mount_root: "/".into(),
            process_relative_path: "/".into(),
            process_path: temp.path().into(),
        });
        external.stale.insert(id);
        external.recovered_processes.insert(id, process);
        let mut sampler =
            ResourceMetricsSampler::new(ResourceMetricsConfig::default(), storage.as_mut())
                .unwrap();
        sampler.external = Some(external);
        let traces = trace_runtime::TraceRuntime::new(Vec::new(), 2);
        let pid = std::process::id();
        let ticks = super::super::read_proc_stat(pid)
            .unwrap()
            .unwrap()
            .start_time_ticks;
        for (start, expected_finals) in [(ticks, 0), (ticks + 1, 1)] {
            let registry = ProcessIdentityManager::with_reserved_block(
                2,
                3,
                [ProcessRecord::new(
                    process,
                    ProcessObservation::host(HostProcessCoordinates::new(pid, start)),
                )],
            )
            .unwrap();
            let drafts = sampler
                .poll_external_finalizations(&traces, &registry, storage.as_mut())
                .unwrap();
            assert_eq!(drafts.len(), expected_finals);
        }
        let trace = storage.get_trace(id).unwrap().unwrap();
        assert_eq!(trace.lifecycle_state, TraceLifecycleState::Completed);
        assert!(trace.timings.completed_at.is_some());
        sampler.finish_external_finalization(id);
        assert!(sampler.external_barrier_ready(id));
    }

    #[test]
    fn startup_stale_binding_persists_degraded_health_and_one_diagnostic() {
        use model_core::ids::{OtelTraceId, ProfileName, TraceName};
        use model_core::trace::{TraceAlertToken, TraceHealth, TraceRecord};
        let temp = tempfile::tempdir().unwrap();
        let config = storage_factory::StorageConfig::sqlite_path(temp.path().join("health.sqlite"));
        let mut storage = storage_factory::open_storage_backend(
            &config,
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        let id = TraceId::new(1);
        storage
            .create_trace(TraceRecord::new(
                id,
                OtelTraceId::from_bytes([1; 16]).unwrap(),
                TraceAlertToken::new([0; 32]),
                ProcessIdentity::new(1),
                TraceName::new("health"),
                ProfileName::new("test"),
                UNIX_EPOCH,
            ))
            .unwrap();
        storage
            .create_external_cgroup_binding(ExternalCgroupBinding::active(
                id,
                ContainerRuntime::Docker,
                NormalizedContainerId::from_lower_hex(ID).unwrap(),
                format!("docker/{ID}"),
                1,
                2,
                HostBootId::from_bytes([1; 16]),
                UNIX_EPOCH,
            ))
            .unwrap();
        let metrics = ResourceMetricsConfig {
            enabled: true,
            existing_container_cgroups: ExistingContainerCgroups::Require,
            ..ResourceMetricsConfig::default()
        };
        ExternalCgroupRuntime::initialize(&metrics, storage.as_mut()).unwrap();
        drop(storage);
        let mut storage = storage_factory::open_storage_backend(
            &config,
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        ExternalCgroupRuntime::initialize(&metrics, storage.as_mut()).unwrap();
        assert_eq!(
            storage.get_trace(id).unwrap().unwrap().health,
            TraceHealth::Degraded
        );
        let diagnostics = storage.list_diagnostics(id).unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0]
                .metadata
                .contains_key("external_cgroup_stale")
        );
    }

    #[test]
    fn legacy_orphan_binding_does_not_block_repeated_startup() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = storage_factory::open_storage_backend(
            &storage_factory::StorageConfig::sqlite_path(temp.path().join("test.sqlite")),
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        let id = TraceId::new(77);
        storage
            .create_external_cgroup_binding(ExternalCgroupBinding::active(
                id,
                ContainerRuntime::Docker,
                NormalizedContainerId::from_lower_hex(ID).unwrap(),
                format!("docker/{ID}"),
                1,
                2,
                HostBootId::from_bytes([1; 16]),
                UNIX_EPOCH,
            ))
            .unwrap();
        let config = ResourceMetricsConfig {
            enabled: true,
            existing_container_cgroups: ExistingContainerCgroups::Require,
            ..ResourceMetricsConfig::default()
        };
        for _ in 0..2 {
            let runtime = ExternalCgroupRuntime::initialize(&config, storage.as_mut())
                .unwrap()
                .unwrap();
            assert!(!runtime.has_pending(id));
        }
        assert!(storage.get_external_cgroup_binding(id).unwrap().is_none());
        assert_eq!(storage.next_diagnostic_id_seed().unwrap(), 2);
    }

    #[test]
    fn reopen_verifies_device_and_inode_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mount = temp.path().join("cgroup");
        let relative = PathBuf::from(format!("docker/{ID}"));
        let boundary = mount.join(&relative);
        fs::create_dir_all(&boundary).unwrap();
        let metadata = fs::metadata(&boundary).unwrap();
        let hierarchy = UnifiedHierarchy {
            mount_point: mount,
            mount_root: PathBuf::from("/"),
            process_relative_path: PathBuf::from("/"),
            process_path: temp.path().join("cgroup"),
        };
        let mut runtime = runtime(hierarchy);
        let binding = ExternalCgroupBinding::active(
            TraceId::new(7),
            ContainerRuntime::Docker,
            NormalizedContainerId::from_lower_hex(ID).unwrap(),
            relative,
            metadata.dev(),
            metadata.ino(),
            HostBootId::from_bytes([1; 16]),
            UNIX_EPOCH,
        );

        assert_eq!(runtime.reopen(&binding), Ok(()));
        assert!(runtime.sources.contains_key(&TraceId::new(7)));

        let mut changed = binding.clone();
        changed.cgroup_inode = metadata.ino().wrapping_add(1);
        assert_eq!(
            runtime.reopen(&changed),
            Err(ExternalBindingStaleReason::BoundaryIdentityChanged)
        );
    }

    #[test]
    fn recovered_process_and_terminal_state_are_removed_with_binding() {
        let temp = tempfile::tempdir().unwrap();
        let hierarchy = UnifiedHierarchy {
            mount_point: temp.path().to_path_buf(),
            mount_root: PathBuf::from("/"),
            process_relative_path: PathBuf::from("/"),
            process_path: temp.path().to_path_buf(),
        };
        let mut runtime = runtime(hierarchy);
        let trace_id = TraceId::new(9);
        let process = ProcessIdentity::new(11);
        runtime.recovered_processes.insert(trace_id, process);
        runtime.recovered_terminal.insert(trace_id);
        runtime.stale.insert(trace_id);

        assert_eq!(
            runtime.recovered_terminal_processes().collect::<Vec<_>>(),
            vec![(trace_id, process)]
        );
        runtime.remove(trace_id);
        assert!(runtime.recovered_processes().next().is_none());
        assert!(!runtime.has_pending(trace_id));
    }
}

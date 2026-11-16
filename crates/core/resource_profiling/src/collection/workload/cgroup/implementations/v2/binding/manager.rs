use crate::collection::workload::cgroup::common::CgroupAvailability;
use crate::collection::workload::cgroup::common::sampler::CgroupSamplerFactory;
use crate::{CgroupScopeSampler, CgroupWorkloadSample, ResourceProfilingBackend, SamplingContext};
// Read-only host container cgroup runtime for attached container traces.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::SystemTime;

use super::CgroupV2BindingPersistence;
use super::{CgroupV2BindingHandle, ContainerCgroupResolver};
use crate::ProfilingError;
use crate::model::cgroup_v2_binding::CgroupV2Binding;
use crate::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use crate::model::cgroup_v2_binding::CgroupV2BindingState;
use crate::model::cgroup_v2_binding::HostBootId;
use crate::{CgroupV2BindingConfig, CgroupV2BindingPolicy as ExistingContainerCgroups};
use linux_cgroup::{UnifiedHierarchy, discover_unified_hierarchy};
use model_core::ids::TraceId;
use model_core::process::{HostProcessCoordinates, ProcessIdentity};

use super::handle::HostBindingEnvironment;

/// One read-only handle to a runtime-owned host container cgroup.
pub(crate) struct CgroupV2BindingSession {
    pub binding: CgroupV2Binding,
    pub backend: CgroupScopeSampler<CgroupV2BindingHandle>,
}

pub(crate) struct CgroupV2BindingManager {
    backend_factory: CgroupSamplerFactory,
    config: CgroupV2BindingConfig,
    procfs_root: std::path::PathBuf,
    hierarchy: Option<UnifiedHierarchy>,
    boot_id: HostBootId,
    sources: BTreeMap<TraceId, CgroupV2BindingSession>,
    stale: BTreeSet<TraceId>,
    recovered_processes: BTreeMap<TraceId, ProcessIdentity>,
    recovered_terminal: BTreeSet<TraceId>,
}

pub(crate) enum CgroupV2BindingAdmission {
    Bound,
    Fallback(String),
}

pub(crate) enum CgroupV2BindingRead {
    Counters(CgroupWorkloadSample),
    BecameStale(CgroupV2BindingStaleReason),
    Retryable(String),
}

pub(crate) struct CgroupV2BindingFinalRead {
    pub binding: CgroupV2Binding,
    pub sample: Option<CgroupWorkloadSample>,
    pub read_error: Option<String>,
}

impl CgroupV2BindingManager {
    pub(crate) fn is_available(&self) -> bool {
        self.hierarchy.is_some()
    }
    pub(crate) fn initialize(
        config: &CgroupV2BindingConfig,
        backend_factory: CgroupSamplerFactory,
        storage: &mut dyn CgroupV2BindingPersistence,
    ) -> Result<Option<Self>, ProfilingError> {
        if config.policy == ExistingContainerCgroups::Disabled {
            return Ok(None);
        }
        let boot_id =
            HostBindingEnvironment::read_host_boot_id(Path::new("/proc")).map_err(|error| {
                ProfilingError::new(
                    "external_cgroup_recovery",
                    format!("cannot read host boot identity: {error}"),
                )
            })?;
        let hierarchy = match discover_unified_hierarchy() {
            Ok(hierarchy) => Some(hierarchy),
            Err(error) if CgroupAvailability::is_unavailable(&error) => {
                tracing::warn!(
                    %error,
                    "cgroup2 discovery failed; external container accounting unavailable"
                );
                None
            }
            Err(error) => {
                return Err(ProfilingError::new(
                    "external_cgroup_discovery",
                    error.to_string(),
                ));
            }
        };
        let records = storage.prepare_v2_binding_recovery()?;
        let recovered_processes = records
            .iter()
            .map(|r| (r.binding.trace_id, r.root))
            .collect();
        let recovered_terminal = records
            .iter()
            .filter(|r| r.terminal)
            .map(|r| r.binding.trace_id)
            .collect();
        let bindings = records.into_iter().map(|r| r.binding).collect::<Vec<_>>();
        let mut runtime = Self {
            backend_factory,
            config: config.clone(),
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
        storage: &mut dyn CgroupV2BindingPersistence,
        bindings: &[CgroupV2Binding],
    ) -> Result<(), ProfilingError> {
        let now = SystemTime::now();
        for binding in bindings {
            if binding.lifecycle_state == CgroupV2BindingState::Stale {
                storage.mark_v2_binding_recovery_stale(
                    binding.trace_id,
                    binding
                        .stale_reason
                        .unwrap_or(CgroupV2BindingStaleReason::BoundaryMissing),
                    now,
                )?;
                self.stale.insert(binding.trace_id);
                continue;
            }
            let reason = if binding.host_boot_id != self.boot_id {
                Some(CgroupV2BindingStaleReason::HostBootChanged)
            } else if self.hierarchy.is_none() {
                Some(CgroupV2BindingStaleReason::BoundaryMissing)
            } else {
                self.reopen(binding).err()
            };
            if let Some(reason) = reason {
                storage.mark_v2_binding_recovery_stale(binding.trace_id, reason, now)?;
                self.stale.insert(binding.trace_id);
            }
        }
        Ok(())
    }

    fn reopen(&mut self, binding: &CgroupV2Binding) -> Result<(), CgroupV2BindingStaleReason> {
        let hierarchy = self
            .hierarchy
            .as_ref()
            .ok_or(CgroupV2BindingStaleReason::BoundaryMissing)?;
        let reopened = ContainerCgroupResolver::reopen_host_container_cgroup(
            hierarchy,
            binding.runtime,
            binding.container_id,
            &binding.relative_path,
        )
        .map_err(HostBindingEnvironment::stale_reason_for_open_error)?;
        if reopened.directory_identity.device != binding.cgroup_device
            || reopened.directory_identity.inode != binding.cgroup_inode
        {
            return Err(CgroupV2BindingStaleReason::BoundaryIdentityChanged);
        }
        self.sources.insert(
            binding.trace_id,
            CgroupV2BindingSession {
                binding: binding.clone(),
                backend: self
                    .backend_factory
                    .bind(
                        *self
                            .recovered_processes
                            .get(&binding.trace_id)
                            .ok_or(CgroupV2BindingStaleReason::ContainerIdentityMismatch)?,
                        reopened,
                    )
                    .map_err(|_| CgroupV2BindingStaleReason::RequiredCounterUnavailable)?,
            },
        );
        Ok(())
    }

    pub(crate) fn admit(
        &mut self,
        trace_id: TraceId,
        root: ProcessIdentity,
        coordinates: &HostProcessCoordinates,
    ) -> Result<CgroupV2BindingAdmission, ProfilingError> {
        let require = self.config.policy == ExistingContainerCgroups::Require;
        if self.sources.contains_key(&trace_id) || self.stale.contains(&trace_id) {
            return Ok(CgroupV2BindingAdmission::Fallback(
                "trace already has an external cgroup binding".to_string(),
            ));
        }
        let Some(hierarchy) = self.hierarchy.as_ref() else {
            let reason = "cgroup v2 unified hierarchy unavailable".to_string();
            if require {
                return Err(ProfilingError::new("external_cgroup_admission", reason));
            }
            return Ok(CgroupV2BindingAdmission::Fallback(reason));
        };
        match ContainerCgroupResolver::bind_host_container_cgroup(
            &self.procfs_root,
            hierarchy,
            coordinates,
            self.config.binding_attempts,
        ) {
            Ok(mut reader) => {
                if let Err(error) = reader.read_counters() {
                    let reason = format!("required external counters unavailable: {error}");
                    if require {
                        return Err(ProfilingError::new("external_cgroup_admission", reason));
                    }
                    return Ok(CgroupV2BindingAdmission::Fallback(reason));
                }
                let binding = CgroupV2Binding::active(
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
                    CgroupV2BindingSession {
                        binding: binding.clone(),
                        backend: self
                            .backend_factory
                            .bind(root, reader)
                            .map_err(|error| ProfilingError::new("cgroup_backend", error))?,
                    },
                );
                Ok(CgroupV2BindingAdmission::Bound)
            }
            Err(error) => {
                let reason = format!("external container cgroup bind failed: {error}");
                if require {
                    return Err(ProfilingError::new("external_cgroup_admission", reason));
                }
                Ok(CgroupV2BindingAdmission::Fallback(reason))
            }
        }
    }

    pub(crate) fn read(
        &mut self,
        storage: &mut dyn CgroupV2BindingPersistence,
        trace_id: TraceId,
        coordinates: Option<&HostProcessCoordinates>,
        context: SamplingContext,
    ) -> Result<CgroupV2BindingRead, ProfilingError> {
        let observed_at = context.observed_at;
        let Some(source) = self.sources.get_mut(&trace_id) else {
            return Ok(CgroupV2BindingRead::Retryable(
                "trace has no active external cgroup source".to_string(),
            ));
        };
        let verification = coordinates
            .ok_or(CgroupV2BindingStaleReason::ContainerIdentityMismatch)
            .and_then(|host| {
                source
                    .backend
                    .source()
                    .verify_membership(&self.procfs_root, host)
                    .map_err(HostBindingEnvironment::stale_reason_for_open_error)
            });
        if let Err(reason) = verification {
            storage.mark_v2_binding_stale(trace_id, reason, observed_at)?;
            self.sources.remove(&trace_id);
            self.stale.insert(trace_id);
            return Ok(CgroupV2BindingRead::BecameStale(reason));
        }
        match source.backend.sample(&(), context) {
            Ok(Some(counters)) => {
                storage.record_v2_binding_success(trace_id, observed_at)?;
                Ok(CgroupV2BindingRead::Counters(counters))
            }
            Ok(None) => Ok(CgroupV2BindingRead::Retryable(
                "cgroup sample unavailable".into(),
            )),
            Err(error) => {
                let failures = storage.record_v2_binding_failure(trace_id, observed_at)?;
                if failures >= self.config.failure_threshold {
                    storage.mark_v2_binding_stale(
                        trace_id,
                        CgroupV2BindingStaleReason::RepeatedReadFailure,
                        observed_at,
                    )?;
                    self.sources.remove(&trace_id);
                    self.stale.insert(trace_id);
                    Ok(CgroupV2BindingRead::BecameStale(
                        CgroupV2BindingStaleReason::RepeatedReadFailure,
                    ))
                } else {
                    Ok(CgroupV2BindingRead::Retryable(error.to_string()))
                }
            }
        }
    }

    pub(crate) fn prepare_final(
        &mut self,
        storage: &mut dyn CgroupV2BindingPersistence,
        trace_id: TraceId,
        context: SamplingContext,
    ) -> Result<Option<CgroupV2BindingFinalRead>, ProfilingError> {
        // A collector admission can fail after preparing a source but before the
        // atomic trace write. It has no durable finalization obligation.
        let Some(binding) = storage.v2_binding(trace_id)? else {
            self.remove(trace_id);
            return Ok(None);
        };
        if let Some(source) = self.sources.get_mut(&trace_id) {
            let (sample, read_error) = match source.backend.sample(&(), context) {
                Ok(sample) => (sample, None),
                Err(error) => (None, Some(error.to_string())),
            };
            return Ok(Some(CgroupV2BindingFinalRead {
                binding,
                sample,
                read_error,
            }));
        }
        if self.stale.contains(&trace_id) {
            return Ok(Some(CgroupV2BindingFinalRead {
                binding,
                sample: None,
                read_error: None,
            }));
        }
        Ok(None)
    }

    pub(crate) fn remove(&mut self, trace_id: TraceId) {
        self.sources.remove(&trace_id);
        self.stale.remove(&trace_id);
        self.recovered_processes.remove(&trace_id);
        self.recovered_terminal.remove(&trace_id);
    }

    pub(crate) fn binding(&self, trace_id: TraceId) -> Option<&CgroupV2Binding> {
        self.sources.get(&trace_id).map(|source| &source.binding)
    }

    pub(crate) fn is_active(&self, trace_id: TraceId) -> bool {
        self.sources.contains_key(&trace_id)
    }

    pub(crate) fn is_stale(&self, trace_id: TraceId) -> bool {
        self.stale.contains(&trace_id)
    }

    pub(crate) fn has_pending(&self, trace_id: TraceId) -> bool {
        self.is_active(trace_id) || self.is_stale(trace_id)
    }

    pub(crate) fn recovered_processes(
        &self,
    ) -> impl Iterator<Item = (TraceId, ProcessIdentity)> + '_ {
        self.recovered_processes
            .iter()
            .map(|(trace_id, process)| (*trace_id, *process))
    }

    pub(crate) fn recovered_terminal_processes(
        &self,
    ) -> impl Iterator<Item = (TraceId, ProcessIdentity)> + '_ {
        self.recovered_terminal.iter().filter_map(|trace_id| {
            self.recovered_processes
                .get(trace_id)
                .map(|process| (*trace_id, *process))
        })
    }
}

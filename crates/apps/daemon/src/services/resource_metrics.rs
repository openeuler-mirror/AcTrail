//! Config-gated process resource sampling from procfs.

#[path = "resource_metrics/cgroup.rs"]
mod cgroup;
#[path = "resource_metrics/external.rs"]
mod external;
#[path = "resource_metrics/external_runtime.rs"]
mod external_runtime;
#[path = "resource_metrics/procfs.rs"]
mod procfs;

#[cfg(test)]
#[path = "resource_metrics/review_tests.rs"]
mod review_tests;

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use config_core::daemon::{ExistingContainerCgroups, ResourceMetricsConfig, ResourceMetricsMode};
use control_contract::reply::ControlError;
use ebpf_collector::procfs::ProcfsIdentityReader;
use model_core::capability::Capability;
use model_core::event::{
    MemoryEventCounters, ResourceAccountingCoverage, ResourceAccountingMethod, ResourcePayload,
    ResourceSampleKind,
};
use model_core::ids::TraceId;
use model_core::process::{HostProcessCoordinates, MembershipState, ProcessIdentity};
use model_core::resource_scope::TraceResourceScope;
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceEntry;

use self::cgroup::{CgroupAdmission, CgroupResourceRuntime};
use self::external_runtime::{
    ExternalAdmission, ExternalCgroupRuntime, ExternalFinalDraft, ExternalRead,
};

use self::procfs::{
    BYTES_PER_KIB, SystemMetrics, SystemUnits, cpu_cores, read_proc_memory, read_proc_stat,
    read_system_metrics,
};

pub(super) const COLLECTOR_NAME: &str = "resource-sampler";

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const PERCENT_MILLIS_SCALE: u128 = 100_000;

#[derive(Clone, Debug)]
pub(super) struct ResourceSampleDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: ResourcePayload,
    pub recovered: bool,
}

pub(super) struct ResourceSamplingFailure {
    pub trace_id: TraceId,
    pub message: String,
    pub recovered: bool,
}

#[derive(Default)]
pub(super) struct ResourceMetricsDrain {
    pub samples: Vec<ResourceSampleDraft>,
    pub failures: Vec<ResourceSamplingFailure>,
}

pub(super) struct ResourceFinalizationDraft {
    pub trace_id: TraceId,
    pub sample: ResourceSampleDraft,
    pub timed_out: bool,
    pub recovered: bool,
}

#[derive(Default)]
pub(super) struct ResourceFinalizationPoll {
    pub waiting: Vec<TraceId>,
    pub ready: Vec<ResourceFinalizationDraft>,
}

pub(super) struct ExternalFinalizationDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: ResourcePayload,
}

enum ExternalSampleOutcome {
    Emitted(ResourceSampleDraft),
    BecameStale,
    Failed(String),
}

pub(super) struct ResourceMetricsSampler {
    config: ResourceMetricsConfig,
    cgroup: Option<CgroupResourceRuntime>,
    external: Option<ExternalCgroupRuntime>,
    cgroup_fallback_reason: Option<String>,
    trace_fallback_reasons: BTreeMap<TraceId, String>,
    next_sample_at: Option<Instant>,
    previous_cpu: BTreeMap<ProcessIdentity, CpuSample>,
    previous_cgroup_cpu: BTreeMap<TraceId, CgroupCpuSample>,
    previous_external_cpu: BTreeMap<TraceId, CgroupCpuSample>,
    finalization_started_at: BTreeMap<TraceId, Instant>,
    finalized_barriers: BTreeSet<TraceId>,
    diagnosed_cgroup_failures: BTreeSet<TraceId>,
    diagnosed_external_failures: BTreeSet<TraceId>,
    units: Option<SystemUnits>,
}

impl ResourceMetricsSampler {
    pub(super) fn new(
        config: ResourceMetricsConfig,
        storage: &mut dyn storage_core::StorageBackend,
    ) -> Result<Self, ControlError> {
        let external = ExternalCgroupRuntime::initialize(&config, storage)?;
        let (cgroup, cgroup_fallback_reason) = CgroupResourceRuntime::initialize(&config, storage)?;
        let now = Instant::now();
        let system_now = SystemTime::now();
        let finalization_started_at = cgroup
            .as_ref()
            .map(|runtime| {
                runtime
                    .waiting_since
                    .iter()
                    .map(|(trace_id, started_at)| {
                        let elapsed = system_now.duration_since(*started_at).unwrap_or_default();
                        (*trace_id, now.checked_sub(elapsed).unwrap_or(now))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let finalized_barriers = cgroup
            .as_ref()
            .map(|runtime| runtime.finalized_barriers.clone())
            .unwrap_or_default();
        let next_sample_at = config
            .enabled
            .then(|| Instant::now() + Duration::from_millis(config.interval_ms));
        Ok(Self {
            config,
            cgroup,
            external,
            cgroup_fallback_reason,
            trace_fallback_reasons: BTreeMap::new(),
            next_sample_at,
            previous_cpu: BTreeMap::new(),
            previous_cgroup_cpu: BTreeMap::new(),
            previous_external_cpu: BTreeMap::new(),
            finalization_started_at,
            finalized_barriers,
            diagnosed_cgroup_failures: BTreeSet::new(),
            diagnosed_external_failures: BTreeSet::new(),
            units: None,
        })
    }

    /// Admit a daemon-controlled, SIGSTOPped launch into its trace scope.
    ///
    /// Scope creation/controller setup may fall back in `auto` mode. Moving the
    /// PID is the commit boundary: any error at or after that point is fatal.
    pub(super) fn admit_stopped_launch(
        &mut self,
        entry: &TraceEntry,
        pid: u32,
        launch_mode: bool,
    ) -> Result<Option<TraceResourceScope>, ControlError> {
        if !self.config.enabled
            || !launch_mode
            || !trace_requests_resource_metrics(entry)
            || self.config.mode == ResourceMetricsMode::Procfs
        {
            return Ok(None);
        }
        let trace_id = entry.trace.trace_id;
        if entry.trace.root_container_id.is_some() {
            self.trace_fallback_reasons.insert(
                trace_id,
                "controlled container/VM launch is outside the host-managed cgroup scope"
                    .to_string(),
            );
            return Ok(None);
        }
        let Some(runtime) = self.cgroup.as_mut() else {
            // Auto-mode startup fallback is already recorded globally.
            return Ok(None);
        };
        match runtime.admit_stopped_launch(trace_id, pid, self.config.mode)? {
            CgroupAdmission::Managed(scope) => Ok(Some(scope)),
            CgroupAdmission::Fallback(reason) => {
                tracing::warn!(%trace_id, %reason, "cgroup launch admission fell back to procfs");
                self.trace_fallback_reasons.insert(trace_id, reason);
                Ok(None)
            }
        }
    }

    pub(super) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ControlError> {
        let runtime = self.cgroup.as_mut().ok_or_else(|| {
            ControlError::new(
                "cgroup_scope_activation",
                "cannot activate a scope without a cgroup runtime",
            )
        })?;
        runtime.activate_scope(scope)?;
        self.trace_fallback_reasons.remove(&scope.trace_id);
        Ok(())
    }

    /// Read-only admission of an attached container's runtime-owned cgroup.
    ///
    /// `require` mode turns an unavailable or unverifiable binding into an
    /// admission failure; otherwise the trace falls back to procfs.
    pub(super) fn admit_external_container(
        &mut self,
        entry: &TraceEntry,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ControlError> {
        if !self.config.enabled
            || self.config.mode == ResourceMetricsMode::Procfs
            || self.config.existing_container_cgroups == ExistingContainerCgroups::Disabled
            || !trace_requests_resource_metrics(entry)
        {
            return Ok(());
        }
        let trace_id = entry.trace.trace_id;
        if entry.trace.root_container_id.is_none() {
            return Ok(());
        }
        let require = self.config.existing_container_cgroups == ExistingContainerCgroups::Require;
        let Some(runtime) = self.external.as_mut() else {
            let reason = "external container cgroup accounting is unavailable".to_string();
            if require {
                return Err(ControlError::new("external_cgroup_admission", reason));
            }
            self.trace_fallback_reasons.insert(trace_id, reason);
            return Ok(());
        };
        match runtime.admit(trace_id, coordinates, require)? {
            ExternalAdmission::Bound => {
                self.trace_fallback_reasons.remove(&trace_id);
                Ok(())
            }
            ExternalAdmission::Fallback(reason) => {
                self.trace_fallback_reasons.insert(trace_id, reason);
                Ok(())
            }
        }
    }

    pub(super) fn poll_finalizations(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
        process_registry: &ProcessIdentityManager,
    ) -> Result<ResourceFinalizationPoll, ControlError> {
        let Some(runtime) = self.cgroup.as_ref() else {
            return Ok(ResourceFinalizationPoll::default());
        };
        let mut candidates = trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| {
                trace.lifecycle_state == model_core::trace::TraceLifecycleState::Draining
                    || trace.lifecycle_state.is_terminal()
            })
            .filter_map(|trace| {
                runtime
                    .scopes
                    .get(&trace.trace_id)
                    .cloned()
                    .map(|scope| (trace.trace_id, trace.root_process_identity, scope, false))
            })
            .collect::<Vec<_>>();
        for (trace_id, process) in &runtime.recovered_processes {
            if candidates
                .iter()
                .any(|(candidate, _, _, _)| candidate == trace_id)
            {
                continue;
            }
            let Some(scope) = runtime.scopes.get(trace_id).cloned() else {
                continue;
            };
            let already_waiting = self.finalization_started_at.contains_key(trace_id);
            let empty = runtime
                .adapter
                .read_populated(&scope.aggregate)
                .map(|populated| !populated)
                .unwrap_or(false);
            if already_waiting || empty || recovered_root_is_gone(*process, process_registry)? {
                candidates.push((*trace_id, *process, scope, true));
            }
        }
        let now = Instant::now();
        let observed_at = SystemTime::now();
        let timeout = Duration::from_millis(self.config.finalization_timeout_ms);
        let mut poll = ResourceFinalizationPoll::default();

        for (trace_id, process, scope, recovered) in candidates {
            let newly_waiting = !self.finalization_started_at.contains_key(&trace_id);
            let started_at = *self.finalization_started_at.entry(trace_id).or_insert(now);
            if newly_waiting {
                poll.waiting.push(trace_id);
            }
            let timed_out = now.saturating_duration_since(started_at) >= timeout;
            let populated = self
                .cgroup
                .as_ref()
                .expect("candidate requires cgroup runtime")
                .adapter
                .read_populated(&scope.aggregate);
            let mut finalization_timed_out = match populated {
                Ok(true) if !timed_out => continue,
                Ok(false) => false,
                Ok(true) => true,
                Err(error) if !timed_out => {
                    tracing::warn!(%trace_id, %error, "cgroup finalization readiness read failed");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        %trace_id,
                        %error,
                        "cgroup finalization readiness read failed at timeout"
                    );
                    true
                }
            };

            let mut sample = match self.collect_cgroup_sample(
                trace_id,
                process,
                &scope,
                now,
                observed_at,
                recovered,
            ) {
                Ok(sample) => sample,
                Err(error) if timed_out => {
                    finalization_timed_out = true;
                    ResourceSampleDraft {
                        trace_id,
                        observed_at,
                        process,
                        recovered,
                        payload: ResourcePayload {
                            scope: "trace".to_string(),
                            subject: trace_id.to_string(),
                            accounting_method: ResourceAccountingMethod::CgroupV2,
                            accounting_coverage: ResourceAccountingCoverage::Partial,
                            sample_kind: ResourceSampleKind::Final,
                            metadata: BTreeMap::from([
                                ("alert".to_string(), "false".to_string()),
                                ("finalization_timeout".to_string(), "true".to_string()),
                                ("finalization_read_error".to_string(), error.message),
                            ]),
                            ..ResourcePayload::default()
                        },
                    }
                }
                Err(error) => {
                    tracing::warn!(%trace_id, error = %error.message,
                        "cgroup final counters unavailable; retrying until deadline");
                    continue;
                }
            };
            sample.payload.sample_kind = ResourceSampleKind::Final;
            if finalization_timed_out {
                sample.payload.accounting_coverage = ResourceAccountingCoverage::Partial;
                sample
                    .payload
                    .metadata
                    .insert("finalization_timeout".to_string(), "true".to_string());
                sample.payload.metadata.insert(
                    "finalization_timeout_ms".to_string(),
                    self.config.finalization_timeout_ms.to_string(),
                );
            }
            poll.ready.push(ResourceFinalizationDraft {
                trace_id,
                sample,
                timed_out: finalization_timed_out,
                recovered,
            });
        }
        Ok(poll)
    }

    pub(super) fn finish_finalization(&mut self, trace_id: TraceId, remove_scope: bool) {
        self.finalized_barriers.insert(trace_id);
        self.diagnosed_cgroup_failures.remove(&trace_id);
        self.finalization_started_at.remove(&trace_id);
        self.previous_cgroup_cpu.remove(&trace_id);
        if let Some(runtime) = self.cgroup.as_mut() {
            runtime.recovered_processes.remove(&trace_id);
            runtime.readers.remove(&trace_id);
            if let Some(paths) = runtime.scopes.remove(&trace_id) {
                if remove_scope {
                    if let Err(error) = runtime.adapter.remove_empty_trace_scope(&paths) {
                        tracing::warn!(%trace_id, %error, "finalized empty cgroup cleanup failed");
                        runtime.queue_cleanup(paths);
                    }
                } else {
                    runtime.queue_cleanup(paths);
                }
            }
        }
    }

    pub(super) fn poll_external_finalizations(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
        process_registry: &ProcessIdentityManager,
        storage: &mut dyn storage_core::StorageBackend,
    ) -> Result<Vec<ExternalFinalizationDraft>, ControlError> {
        let Some(runtime) = self.external.as_mut() else {
            return Ok(Vec::new());
        };
        let mut terminal_traces = trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| trace.lifecycle_state.is_terminal())
            .map(|trace| (trace.trace_id, trace.root_process_identity))
            .collect::<BTreeMap<_, _>>();
        for (trace_id, process) in runtime.recovered_terminal_processes() {
            terminal_traces.entry(trace_id).or_insert(process);
        }
        for (trace_id, process) in runtime.recovered_processes() {
            use model_core::external_cgroup::ExternalBindingStaleReason;
            if trace_runtime.get_trace(trace_id).is_some() {
                continue;
            }
            let boot_changed = storage
                .get_external_cgroup_binding(trace_id)
                .map_err(|e| ControlError::new(e.stage, e.message))?
                .is_some_and(|binding| {
                    binding.stale_reason == Some(ExternalBindingStaleReason::HostBootChanged)
                });
            let root_gone = match recovered_root_is_gone(process, process_registry) {
                Ok(gone) => gone,
                Err(error) => {
                    tracing::warn!(%trace_id, message = %error.message, "cannot reconcile recovered container process; retrying");
                    false
                }
            };
            if !boot_changed && !root_gone {
                continue;
            }
            if let Some(mut trace) = storage
                .get_trace(trace_id)
                .map_err(|e| ControlError::new(e.stage, e.message))?
            {
                if !trace.lifecycle_state.is_terminal() {
                    trace.lifecycle_state = model_core::trace::TraceLifecycleState::Completed;
                    trace.timings.completed_at = Some(SystemTime::now());
                    trace.health = model_core::trace::TraceHealth::Degraded;
                    storage
                        .create_trace(trace)
                        .map_err(|e| ControlError::new(e.stage, e.message))?;
                }
            }
            terminal_traces.entry(trace_id).or_insert(process);
        }
        let mut drafts = Vec::new();
        for (trace_id, process) in terminal_traces {
            let Some(final_draft) = runtime.prepare_final(storage, trace_id)? else {
                continue;
            };
            drafts.push(ExternalFinalizationDraft {
                trace_id,
                observed_at: SystemTime::now(),
                process,
                payload: build_external_final_payload(final_draft),
            });
        }
        Ok(drafts)
    }

    pub(super) fn finish_external_finalization(&mut self, trace_id: TraceId) {
        if let Some(runtime) = self.external.as_mut() {
            runtime.remove(trace_id);
        }
        self.previous_external_cpu.remove(&trace_id);
        self.diagnosed_external_failures.remove(&trace_id);
        self.trace_fallback_reasons.remove(&trace_id);
    }

    pub(super) fn external_binding(
        &self,
        trace_id: TraceId,
    ) -> Option<model_core::external_cgroup::ExternalCgroupBinding> {
        self.external.as_ref()?.binding(trace_id).cloned()
    }

    pub(super) fn prune_forgotten(&mut self, traces: &trace_runtime::TraceRuntime) {
        self.finalized_barriers
            .retain(|id| traces.get_trace(*id).is_some());
        self.trace_fallback_reasons
            .retain(|id, _| traces.get_trace(*id).is_some());
    }

    pub(super) fn resource_barrier_ready(&self, trace_id: TraceId) -> bool {
        self.cgroup
            .as_ref()
            .is_none_or(|runtime| !runtime.scopes.contains_key(&trace_id))
    }

    pub(super) fn completed_managed_barrier(&self, trace_id: TraceId) -> bool {
        self.finalized_barriers.contains(&trace_id)
    }

    pub(super) fn has_pending_barrier(&self, trace_id: TraceId) -> bool {
        self.cgroup
            .as_ref()
            .is_some_and(|runtime| runtime.scopes.contains_key(&trace_id))
    }

    pub(super) fn external_barrier_ready(&self, trace_id: TraceId) -> bool {
        self.external
            .as_ref()
            .is_none_or(|runtime| !runtime.has_pending(trace_id))
    }

    pub(super) fn force_pending_finalizations_due(&mut self) {
        let elapsed = Duration::from_millis(self.config.finalization_timeout_ms);
        let due = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
        for started_at in self.finalization_started_at.values_mut() {
            *started_at = due;
        }
    }

    pub(super) fn poll_timeout(&self) -> Option<Duration> {
        let next_sample_at = self.next_sample_at?;
        Some(next_sample_at.saturating_duration_since(Instant::now()))
    }

    pub(super) fn drain_due(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
        process_registry: &ProcessIdentityManager,
        storage: &mut dyn storage_core::StorageBackend,
    ) -> Result<ResourceMetricsDrain, ControlError> {
        if let Some(runtime) = self.cgroup.as_mut() {
            runtime.cleanup_abandoned_admissions();
        }
        let Some(next_sample_at) = self.next_sample_at else {
            return Ok(ResourceMetricsDrain::default());
        };
        let now = Instant::now();
        if now < next_sample_at {
            return Ok(ResourceMetricsDrain::default());
        }
        self.next_sample_at = Some(now + Duration::from_millis(self.config.interval_ms));
        self.collect_samples(
            trace_runtime,
            process_registry,
            storage,
            now,
            SystemTime::now(),
        )
    }

    fn collect_samples(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
        process_registry: &ProcessIdentityManager,
        storage: &mut dyn storage_core::StorageBackend,
        sampled_at: Instant,
        observed_at: SystemTime,
    ) -> Result<ResourceMetricsDrain, ControlError> {
        let units = self.units()?;
        let mut drafts = Vec::new();
        let mut failures = Vec::new();
        let mut active_identities = BTreeSet::new();
        for trace in trace_runtime.list_trace_records() {
            let Some(entry) = trace_runtime.get_trace(trace.trace_id) else {
                continue;
            };
            if !trace_requests_resource_metrics(entry) {
                continue;
            }
            let cgroup_scope = self
                .cgroup
                .as_ref()
                .and_then(|runtime| runtime.scopes.get(&trace.trace_id))
                .cloned();
            if let Some(scope) = cgroup_scope {
                match self.collect_cgroup_sample(
                    trace.trace_id,
                    entry.trace.root_process_identity,
                    &scope,
                    sampled_at,
                    observed_at,
                    false,
                ) {
                    Ok(draft) => {
                        self.diagnosed_cgroup_failures.remove(&trace.trace_id);
                        drafts.push(draft);
                    }
                    Err(error) => {
                        if self.diagnosed_cgroup_failures.insert(trace.trace_id) {
                            failures.push(ResourceSamplingFailure {
                                trace_id: trace.trace_id,
                                message: format!("{}: {}", error.code, error.message),
                                recovered: false,
                            });
                        }
                    }
                }
                continue;
            }
            if self
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(trace.trace_id))
            {
                let outcome = self.collect_external_sample(
                    trace.trace_id,
                    entry.trace.root_process_identity,
                    storage,
                    sampled_at,
                    observed_at,
                    false,
                );
                if self.record_external_outcome(
                    trace.trace_id,
                    false,
                    outcome,
                    &mut drafts,
                    &mut failures,
                ) {
                    continue;
                }
            }
            let identities = sample_identities(entry, self.config.include_children);
            active_identities.extend(identities.iter().cloned());
            if let Some(mut draft) = self.collect_trace_sample(
                entry,
                identities,
                process_registry,
                sampled_at,
                observed_at,
                units,
            )? {
                if self
                    .external
                    .as_ref()
                    .is_some_and(|runtime| runtime.is_stale(trace.trace_id))
                {
                    draft.payload.metadata.insert(
                        "fallback_from".to_string(),
                        "external_container_cgroup".to_string(),
                    );
                }
                drafts.push(draft);
            }
        }
        let recovered_scopes = self
            .cgroup
            .as_ref()
            .map(|runtime| {
                runtime
                    .recovered_processes
                    .iter()
                    .filter_map(|(trace_id, process)| {
                        if trace_runtime.get_trace(*trace_id).is_some() {
                            return None;
                        }
                        runtime
                            .scopes
                            .get(trace_id)
                            .cloned()
                            .map(|scope| (*trace_id, *process, scope))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (trace_id, process, scope) in recovered_scopes {
            match self.collect_cgroup_sample(
                trace_id,
                process,
                &scope,
                sampled_at,
                observed_at,
                true,
            ) {
                Ok(draft) => {
                    self.diagnosed_cgroup_failures.remove(&trace_id);
                    drafts.push(draft);
                }
                Err(error) => {
                    if self.diagnosed_cgroup_failures.insert(trace_id) {
                        failures.push(ResourceSamplingFailure {
                            trace_id,
                            message: format!("{}: {}", error.code, error.message),
                            recovered: true,
                        });
                    }
                }
            }
        }
        let recovered_external = self
            .external
            .as_ref()
            .map(|runtime| {
                runtime
                    .recovered_processes()
                    .filter(|(trace_id, _)| trace_runtime.get_trace(*trace_id).is_none())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (trace_id, process) in recovered_external {
            let external_active = self
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(trace_id));
            if external_active {
                let outcome = self.collect_external_sample(
                    trace_id,
                    process,
                    storage,
                    sampled_at,
                    observed_at,
                    true,
                );
                if self.record_external_outcome(trace_id, true, outcome, &mut drafts, &mut failures)
                {
                    continue;
                }
            }
            if self
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_stale(trace_id))
            {
                let identities = recovered_sample_identities(
                    storage,
                    trace_id,
                    process,
                    self.config.include_children,
                )?;
                active_identities.extend(identities.iter().copied());
                if let Some(mut draft) = self.collect_procfs_sample(
                    trace_id,
                    process,
                    identities,
                    process_registry,
                    sampled_at,
                    observed_at,
                    units,
                    true,
                    Some("external container cgroup binding is stale".to_string()),
                )? {
                    draft.payload.metadata.insert(
                        "fallback_from".to_string(),
                        "external_container_cgroup".to_string(),
                    );
                    drafts.push(draft);
                }
            }
        }
        self.previous_cpu
            .retain(|identity, _| active_identities.contains(identity));
        self.previous_cgroup_cpu.retain(|trace_id, _| {
            self.cgroup
                .as_ref()
                .is_some_and(|runtime| runtime.scopes.contains_key(trace_id))
        });
        self.previous_external_cpu.retain(|trace_id, _| {
            self.external
                .as_ref()
                .is_some_and(|runtime| runtime.is_active(*trace_id))
        });
        Ok(ResourceMetricsDrain {
            samples: drafts,
            failures,
        })
    }

    fn collect_cgroup_sample(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        scope: &linux_platform::cgroup_v2::TraceScopePaths,
        sampled_at: Instant,
        observed_at: SystemTime,
        recovered: bool,
    ) -> Result<ResourceSampleDraft, ControlError> {
        let counters = self
            .cgroup
            .as_mut()
            .expect("managed scope requires initialized cgroup runtime")
            .read_counters(trace_id, &scope.aggregate)
            .map_err(|error| {
                ControlError::new("resource_metrics_cgroup_read", error.to_string())
            })?;
        let cpu_rate =
            self.cgroup_cpu_percent_millis(trace_id, counters.cpu.usage_usec, sampled_at);
        let cpu_percent_millis = cpu_rate.map(|rate| rate.percent_millis);
        let process_rss = if self.config.memory_alert_rss_kb.is_some() {
            Some(self.collect_cgroup_process_rss(&scope.aggregate)?)
        } else {
            None
        };
        let cpu_alert = self
            .config
            .cpu_alert_percent_millis
            .zip(cpu_percent_millis)
            .is_some_and(|(threshold, observed)| observed >= threshold);
        let memory_rss_alert = self
            .config
            .memory_alert_rss_kb
            .zip(process_rss.map(|sample| sample.rss_sum_kb))
            .is_some_and(|(threshold, observed)| observed >= threshold);
        let memory_current_alert = self
            .config
            .memory_alert_current_bytes
            .is_some_and(|threshold| counters.memory_current_bytes >= threshold);
        let memory_alert = memory_rss_alert || memory_current_alert;
        let mut metadata = BTreeMap::from([
            (
                "cgroup_relative_path".to_string(),
                scope.relative_path.display().to_string(),
            ),
            ("alert".to_string(), (cpu_alert || memory_alert).to_string()),
            (
                "cpu_rate_basis".to_string(),
                "delta(cpu.stat.usage_usec)/monotonic_elapsed_usec".to_string(),
            ),
        ]);
        if let Some(rate) = cpu_rate {
            metadata.insert(
                "cpu_rate_interval_usec".to_string(),
                rate.interval_usec.to_string(),
            );
        }
        if let Some(sample) = process_rss {
            metadata.insert(
                "process_rss_candidate_processes".to_string(),
                sample.candidate_processes.to_string(),
            );
            metadata.insert(
                "process_rss_sampled_processes".to_string(),
                sample.sampled_processes.to_string(),
            );
            metadata.insert(
                "memory_alert_rss_kb".to_string(),
                self.config
                    .memory_alert_rss_kb
                    .expect("RSS collection requires configured threshold")
                    .to_string(),
            );
        }
        if let Some(threshold) = self.config.memory_alert_current_bytes {
            metadata.insert(
                "memory_alert_current_bytes".to_string(),
                threshold.to_string(),
            );
        }
        for (key, value) in &counters.cpu.unknown {
            metadata.insert(format!("cpu.stat.{key}"), value.clone());
        }
        if let Some(io) = &counters.io {
            for (key, value) in &io.unknown {
                metadata.insert(format!("io.stat.{key}"), value.clone());
            }
        } else {
            metadata.insert(
                "missing_optional_file.io.stat".to_string(),
                "true".to_string(),
            );
        }
        if counters.pids_current.is_none() {
            metadata.insert(
                "missing_optional_file.pids.current".to_string(),
                "true".to_string(),
            );
        }
        if self.config.include_system {
            add_system_metadata(&mut metadata, read_system_metrics()?);
        }
        let mut payload = ResourcePayload {
            scope: "trace".to_string(),
            subject: trace_id.to_string(),
            accounting_method: ResourceAccountingMethod::CgroupV2,
            accounting_coverage: ResourceAccountingCoverage::Exact,
            sample_kind: ResourceSampleKind::Periodic,
            cpu_percent_millis,
            rss_kb: process_rss.map(|sample| sample.rss_sum_kb),
            process_rss_sum_kb: process_rss.map(|sample| sample.rss_sum_kb),
            metadata,
            ..ResourcePayload::default()
        };
        apply_cgroup_counters(&mut payload, counters);
        Ok(ResourceSampleDraft {
            trace_id,
            observed_at,
            process,
            recovered,
            payload,
        })
    }

    fn collect_cgroup_process_rss(
        &mut self,
        aggregate: &std::path::Path,
    ) -> Result<CgroupProcessRss, ControlError> {
        let pids = linux_platform::cgroup_v2::read_cgroup_subtree_pids(aggregate)
            .map_err(|error| ControlError::new("resource_metrics_cgroup_rss", error.to_string()))?;
        let candidate_processes = pids.len();
        let units = self.units()?;
        let mut rss_sum_kb = 0_u64;
        let mut sampled_processes = 0_usize;
        for pid in pids {
            let Some(memory) = read_proc_memory(pid, units.page_size_kb)? else {
                continue;
            };
            rss_sum_kb = rss_sum_kb.saturating_add(memory.rss_kb);
            sampled_processes += 1;
        }
        Ok(CgroupProcessRss {
            rss_sum_kb,
            candidate_processes,
            sampled_processes,
        })
    }

    /// Shared live/recovered disposition. Only a stale source falls through to procfs.
    fn record_external_outcome(
        &mut self,
        trace_id: TraceId,
        recovered: bool,
        outcome: Result<ExternalSampleOutcome, ControlError>,
        drafts: &mut Vec<ResourceSampleDraft>,
        failures: &mut Vec<ResourceSamplingFailure>,
    ) -> bool {
        let (message, consumed) = match outcome {
            Ok(ExternalSampleOutcome::Emitted(draft)) => {
                self.diagnosed_external_failures.remove(&trace_id);
                drafts.push(draft);
                return true;
            }
            Ok(ExternalSampleOutcome::BecameStale) => (
                "external cgroup identity lost; using procfs fallback".to_string(),
                false,
            ),
            Ok(ExternalSampleOutcome::Failed(message)) => (message, true),
            Err(error) => (format!("{}: {}", error.code, error.message), true),
        };
        if self.diagnosed_external_failures.insert(trace_id) {
            failures.push(ResourceSamplingFailure {
                trace_id,
                message,
                recovered,
            });
        }
        consumed
    }

    fn collect_external_sample(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        storage: &mut dyn storage_core::StorageBackend,
        sampled_at: Instant,
        observed_at: SystemTime,
        recovered: bool,
    ) -> Result<ExternalSampleOutcome, ControlError> {
        let Some(runtime) = self.external.as_mut() else {
            return Ok(ExternalSampleOutcome::Failed(
                "external container cgroup runtime is unavailable".to_string(),
            ));
        };
        let binding = runtime.binding(trace_id).cloned();
        let coordinates = storage
            .get_process_record(process)
            .map_err(|e| ControlError::new(e.stage, e.message))?
            .and_then(|record| record.host);
        let rss_path = runtime.boundary_path(trace_id);
        let read = runtime.read(
            storage,
            trace_id,
            coordinates.as_ref(),
            observed_at,
            self.config.external_cgroup_failure_threshold,
        )?;
        match read {
            ExternalRead::Counters(counters) => {
                let process_rss = if self.config.memory_alert_rss_kb.is_some() {
                    rss_path
                        .as_ref()
                        .map(|path| self.collect_cgroup_process_rss(path))
                        .transpose()?
                } else {
                    None
                };
                let cpu_rate =
                    self.external_cpu_percent_millis(trace_id, counters.cpu.usage_usec, sampled_at);
                let cpu_percent_millis = cpu_rate.map(|rate| rate.percent_millis);
                let cpu_alert = self
                    .config
                    .cpu_alert_percent_millis
                    .zip(cpu_percent_millis)
                    .is_some_and(|(threshold, observed)| observed >= threshold);
                let memory_alert = self
                    .config
                    .memory_alert_current_bytes
                    .is_some_and(|threshold| counters.memory_current_bytes >= threshold)
                    || self
                        .config
                        .memory_alert_rss_kb
                        .zip(process_rss.map(|rss| rss.rss_sum_kb))
                        .is_some_and(|(threshold, rss)| rss >= threshold);
                let mut metadata = BTreeMap::new();
                if let Some(binding) = &binding {
                    metadata.insert("ownership".to_string(), "runtime-owned".to_string());
                    metadata.insert("runtime".to_string(), binding.runtime.as_str().to_string());
                    metadata.insert("container_id".to_string(), binding.container_id.to_string());
                    metadata.insert(
                        "relative_path".to_string(),
                        binding.relative_path.display().to_string(),
                    );
                    metadata.insert(
                        "memory_peak_window".to_string(),
                        "cgroup_lifetime".to_string(),
                    );
                    metadata.insert(
                        "attachment_time_unix_seconds".to_string(),
                        binding
                            .created_at
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|duration| duration.as_secs().to_string())
                            .unwrap_or_default(),
                    );
                }
                metadata.insert("alert".to_string(), (cpu_alert || memory_alert).to_string());
                metadata.insert(
                    "cpu_rate_basis".to_string(),
                    "delta(cpu.stat.usage_usec)/monotonic_elapsed_usec".to_string(),
                );
                if let Some(rate) = cpu_rate {
                    metadata.insert(
                        "cpu_rate_interval_usec".to_string(),
                        rate.interval_usec.to_string(),
                    );
                }
                if let Some(threshold) = self.config.memory_alert_current_bytes {
                    metadata.insert(
                        "memory_alert_current_bytes".to_string(),
                        threshold.to_string(),
                    );
                }
                if let Some(sample) = process_rss {
                    metadata.insert(
                        "memory_alert_rss_kb".to_string(),
                        self.config.memory_alert_rss_kb.unwrap().to_string(),
                    );
                    metadata.insert(
                        "process_rss_candidate_processes".to_string(),
                        sample.candidate_processes.to_string(),
                    );
                    metadata.insert(
                        "process_rss_sampled_processes".to_string(),
                        sample.sampled_processes.to_string(),
                    );
                }
                let mut payload = ResourcePayload {
                    scope: "container".to_string(),
                    rss_kb: process_rss.map(|rss| rss.rss_sum_kb),
                    process_rss_sum_kb: process_rss.map(|rss| rss.rss_sum_kb),
                    subject: binding
                        .as_ref()
                        .map(|binding| format!("container:{}", binding.container_id))
                        .unwrap_or_else(|| "container".to_string()),
                    accounting_method: ResourceAccountingMethod::CgroupV2,
                    accounting_coverage: ResourceAccountingCoverage::BroaderThanTrace,
                    sample_kind: ResourceSampleKind::Periodic,
                    cpu_percent_millis,
                    metadata,
                    ..ResourcePayload::default()
                };
                apply_cgroup_counters(&mut payload, counters);
                Ok(ExternalSampleOutcome::Emitted(ResourceSampleDraft {
                    trace_id,
                    observed_at,
                    process,
                    recovered,
                    payload,
                }))
            }
            ExternalRead::BecameStale(reason) => {
                tracing::warn!(
                    %trace_id,
                    reason = reason.as_storage_str(),
                    "external container cgroup binding became stale; falling back to procfs"
                );
                Ok(ExternalSampleOutcome::BecameStale)
            }
            ExternalRead::Retryable(message) => Ok(ExternalSampleOutcome::Failed(message)),
        }
    }

    fn collect_trace_sample(
        &mut self,
        entry: &TraceEntry,
        identities: Vec<ProcessIdentity>,
        process_registry: &ProcessIdentityManager,
        sampled_at: Instant,
        observed_at: SystemTime,
        units: SystemUnits,
    ) -> Result<Option<ResourceSampleDraft>, ControlError> {
        let fallback_reason = if self.config.mode != ResourceMetricsMode::Procfs {
            Some(
                self.trace_fallback_reasons
                    .get(&entry.trace.trace_id)
                    .cloned()
                    .or_else(|| self.cgroup_fallback_reason.clone())
                    .unwrap_or_else(|| "trace_has_no_managed_cgroup_scope".to_string()),
            )
        } else {
            None
        };
        self.collect_procfs_sample(
            entry.trace.trace_id,
            entry.trace.root_process_identity,
            identities,
            process_registry,
            sampled_at,
            observed_at,
            units,
            false,
            fallback_reason,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_procfs_sample(
        &mut self,
        trace_id: TraceId,
        root: ProcessIdentity,
        identities: Vec<ProcessIdentity>,
        process_registry: &ProcessIdentityManager,
        sampled_at: Instant,
        observed_at: SystemTime,
        units: SystemUnits,
        recovered: bool,
        fallback_reason: Option<String>,
    ) -> Result<Option<ResourceSampleDraft>, ControlError> {
        let mut total_cpu_percent_millis = 0_u64;
        let mut total_rss_kb = 0_u64;
        let mut total_virtual_memory_kb = 0_u64;
        let mut sampled_processes = 0_usize;
        let mut root_threads = None;
        let mut root_comm = None;
        let root_host_pid = process_registry
            .record(root)
            .and_then(|record| record.host.as_ref())
            .map(|host| host.pid);

        for identity in identities {
            let Some(host) = process_registry
                .record(identity)
                .and_then(|record| record.host.as_ref())
            else {
                continue;
            };
            let Some(expected_start_time_ticks) =
                ProcfsIdentityReader.expected_start_time_ticks(host)
            else {
                continue;
            };
            let Some(stat) = read_proc_stat(host.pid)? else {
                continue;
            };
            if stat.exited || stat.start_time_ticks != expected_start_time_ticks {
                continue;
            }
            let Some(memory) = read_proc_memory(host.pid, units.page_size_kb)? else {
                continue;
            };
            // Check again after the separate statm read: the PID may have exited
            // and been reused between those reads. Never charge its replacement.
            let Some(after) = read_proc_stat(host.pid)? else {
                continue;
            };
            if after.exited || after.start_time_ticks != expected_start_time_ticks {
                continue;
            }
            total_cpu_percent_millis = total_cpu_percent_millis.saturating_add(
                self.cpu_percent_millis(&identity, stat.total_cpu_ticks, sampled_at, units),
            );
            total_rss_kb = total_rss_kb.saturating_add(memory.rss_kb);
            total_virtual_memory_kb =
                total_virtual_memory_kb.saturating_add(memory.virtual_memory_kb);
            sampled_processes += 1;
            if identity == root {
                root_threads = Some(stat.threads);
                root_comm = Some(stat.comm);
            }
        }

        if sampled_processes == 0 {
            return Ok(None);
        }

        let mut metadata = BTreeMap::new();
        if let Some(comm) = root_comm {
            metadata.insert("comm".to_string(), comm);
        }
        metadata.insert(
            "sampled_processes".to_string(),
            sampled_processes.to_string(),
        );
        metadata.insert(
            "children".to_string(),
            sampled_processes.saturating_sub(1).to_string(),
        );
        metadata.insert(
            "threads".to_string(),
            root_threads.unwrap_or_default().to_string(),
        );
        metadata.insert(
            "cpu_percent".to_string(),
            format_percent_millis(total_cpu_percent_millis),
        );
        metadata.insert(
            "cpu_percent_millis".to_string(),
            total_cpu_percent_millis.to_string(),
        );
        metadata.insert("cpu_cores".to_string(), cpu_cores()?);
        metadata.insert("rss_kb".to_string(), total_rss_kb.to_string());
        metadata.insert(
            "rss_mb".to_string(),
            (total_rss_kb / BYTES_PER_KIB).to_string(),
        );
        metadata.insert(
            "virtual_memory_kb".to_string(),
            total_virtual_memory_kb.to_string(),
        );
        metadata.insert(
            "virtual_memory_mb".to_string(),
            (total_virtual_memory_kb / BYTES_PER_KIB).to_string(),
        );
        metadata.insert(
            "include_children".to_string(),
            self.config.include_children.to_string(),
        );
        if let Some(fallback_reason) = fallback_reason {
            metadata.insert("cgroup_fallback_reason".to_string(), fallback_reason);
        }
        metadata.insert(
            "alert".to_string(),
            resource_alert(&self.config, total_cpu_percent_millis, total_rss_kb).to_string(),
        );
        if self.config.include_system {
            add_system_metadata(&mut metadata, read_system_metrics()?);
        }

        Ok(Some(ResourceSampleDraft {
            trace_id,
            observed_at,
            process: root,
            recovered,
            payload: ResourcePayload {
                scope: if self.config.include_children {
                    "process_tree".to_string()
                } else {
                    "process".to_string()
                },
                subject: root_host_pid
                    .map(|pid| format!("pid:{pid}"))
                    .unwrap_or_else(|| root.to_string()),
                accounting_method: ResourceAccountingMethod::ProcfsRssSum,
                accounting_coverage: ResourceAccountingCoverage::Partial,
                sample_kind: ResourceSampleKind::Periodic,
                cpu_percent_millis: Some(total_cpu_percent_millis),
                rss_kb: Some(total_rss_kb),
                virtual_memory_kb: Some(total_virtual_memory_kb),
                process_rss_sum_kb: Some(total_rss_kb),
                metadata,
                ..ResourcePayload::default()
            },
        }))
    }

    fn cpu_percent_millis(
        &mut self,
        identity: &ProcessIdentity,
        total_cpu_ticks: u64,
        sampled_at: Instant,
        units: SystemUnits,
    ) -> u64 {
        let current = CpuSample {
            total_cpu_ticks,
            sampled_at,
        };
        let percent = self
            .previous_cpu
            .get(identity)
            .and_then(|previous| {
                let elapsed = sampled_at.checked_duration_since(previous.sampled_at)?;
                let elapsed_nanos = elapsed.as_nanos();
                if elapsed_nanos == u128::default() || total_cpu_ticks < previous.total_cpu_ticks {
                    return None;
                }
                let tick_delta = u128::from(total_cpu_ticks - previous.total_cpu_ticks);
                let numerator = tick_delta
                    .saturating_mul(NANOS_PER_SECOND)
                    .saturating_mul(PERCENT_MILLIS_SCALE);
                let denominator =
                    u128::from(units.clock_ticks_per_second).saturating_mul(elapsed_nanos);
                u64::try_from(numerator / denominator).ok()
            })
            .unwrap_or_default();
        self.previous_cpu.insert(identity.clone(), current);
        percent
    }

    fn cgroup_cpu_percent_millis(
        &mut self,
        trace_id: TraceId,
        usage_usec: u64,
        sampled_at: Instant,
    ) -> Option<CgroupCpuRate> {
        let rate = cgroup_cpu_rate(&self.previous_cgroup_cpu, trace_id, usage_usec, sampled_at);
        self.previous_cgroup_cpu.insert(
            trace_id,
            CgroupCpuSample {
                usage_usec,
                sampled_at,
            },
        );
        rate
    }

    fn external_cpu_percent_millis(
        &mut self,
        trace_id: TraceId,
        usage_usec: u64,
        sampled_at: Instant,
    ) -> Option<CgroupCpuRate> {
        let rate = cgroup_cpu_rate(
            &self.previous_external_cpu,
            trace_id,
            usage_usec,
            sampled_at,
        );
        self.previous_external_cpu.insert(
            trace_id,
            CgroupCpuSample {
                usage_usec,
                sampled_at,
            },
        );
        rate
    }

    fn units(&mut self) -> Result<SystemUnits, ControlError> {
        if self.units.is_none() {
            self.units = Some(
                SystemUnits::read()
                    .map_err(|message| ControlError::new("resource_metrics_sysconf", message))?,
            );
        }
        self.units
            .ok_or_else(|| ControlError::new("resource_metrics_sysconf", "missing system units"))
    }
}

#[derive(Clone, Copy)]
struct CpuSample {
    total_cpu_ticks: u64,
    sampled_at: Instant,
}

#[derive(Clone, Copy)]
struct CgroupCpuSample {
    usage_usec: u64,
    sampled_at: Instant,
}

#[derive(Clone, Copy)]
struct CgroupCpuRate {
    percent_millis: u64,
    interval_usec: u128,
}

#[derive(Clone, Copy)]
struct CgroupProcessRss {
    rss_sum_kb: u64,
    candidate_processes: usize,
    sampled_processes: usize,
}

fn convert_memory_events(events: linux_platform::cgroup_v2::MemoryEvents) -> MemoryEventCounters {
    MemoryEventCounters {
        low: events.low,
        high: events.high,
        max: events.max,
        oom: events.oom,
        oom_kill: events.oom_kill,
        oom_group_kill: events.oom_group_kill,
    }
}

fn cgroup_cpu_rate(
    previous: &BTreeMap<TraceId, CgroupCpuSample>,
    trace_id: TraceId,
    usage_usec: u64,
    sampled_at: Instant,
) -> Option<CgroupCpuRate> {
    previous.get(&trace_id).and_then(|previous| {
        let elapsed_usec = sampled_at
            .checked_duration_since(previous.sampled_at)?
            .as_micros();
        if elapsed_usec == 0 || usage_usec < previous.usage_usec {
            return None;
        }
        let usage_delta = u128::from(usage_usec - previous.usage_usec);
        let percent_millis =
            u64::try_from(usage_delta.saturating_mul(PERCENT_MILLIS_SCALE) / elapsed_usec).ok()?;
        Some(CgroupCpuRate {
            percent_millis,
            interval_usec: elapsed_usec,
        })
    })
}

fn apply_cgroup_counters(
    payload: &mut ResourcePayload,
    counters: linux_platform::cgroup_v2::CgroupCounters,
) {
    payload.memory_current_bytes = Some(counters.memory_current_bytes);
    payload.memory_peak_bytes = counters.memory_peak_bytes;
    payload.memory_anon_bytes = counters.memory_anon_bytes;
    payload.memory_file_bytes = counters.memory_file_bytes;
    payload.memory_swap_current_bytes = counters.memory_swap_current_bytes;
    payload.memory_events = counters.memory_events.map(convert_memory_events);
    payload.memory_events_local = counters.memory_events_local.map(convert_memory_events);
    payload.cpu_usage_usec = Some(counters.cpu.usage_usec);
    payload.cpu_user_usec = counters.cpu.user_usec;
    payload.cpu_system_usec = counters.cpu.system_usec;
    payload.cpu_nr_throttled = counters.cpu.nr_throttled;
    payload.cpu_throttled_usec = counters.cpu.throttled_usec;
    payload.io_read_bytes = counters.io.as_ref().and_then(|io| io.read_bytes);
    payload.io_write_bytes = counters.io.as_ref().and_then(|io| io.write_bytes);
    payload.pids_current = counters.pids_current;
    payload.pids_peak = counters.pids_peak;
}

fn build_external_final_payload(draft: ExternalFinalDraft) -> ResourcePayload {
    let binding = &draft.binding;
    let mut metadata = BTreeMap::new();
    metadata.insert("ownership".to_string(), "runtime-owned".to_string());
    metadata.insert("runtime".to_string(), binding.runtime.as_str().to_string());
    metadata.insert("container_id".to_string(), binding.container_id.to_string());
    metadata.insert(
        "relative_path".to_string(),
        binding.relative_path.display().to_string(),
    );
    metadata.insert(
        "memory_peak_window".to_string(),
        "cgroup_lifetime".to_string(),
    );
    if let Some(seconds) = unix_seconds(binding.created_at) {
        metadata.insert(
            "attachment_time_unix_seconds".to_string(),
            seconds.to_string(),
        );
    }
    if let Some(reason) = binding.stale_reason {
        metadata.insert(
            "fallback_from".to_string(),
            "external_container_cgroup".to_string(),
        );
        metadata.insert(
            "stale_reason".to_string(),
            reason.as_storage_str().to_string(),
        );
    }
    if let Some(last_good_at) = binding.last_good_at {
        if let Some(seconds) = unix_seconds(last_good_at) {
            metadata.insert("last_good_at_unix_seconds".to_string(), seconds.to_string());
        }
    }
    if let Some(error) = &draft.read_error {
        metadata.insert("finalization_read_error".to_string(), error.clone());
    }
    let has_counters = draft.counters.is_some();
    let mut payload = ResourcePayload {
        scope: "container".to_string(),
        subject: format!("container:{}", binding.container_id),
        accounting_method: ResourceAccountingMethod::CgroupV2,
        accounting_coverage: if has_counters {
            ResourceAccountingCoverage::BroaderThanTrace
        } else {
            ResourceAccountingCoverage::Partial
        },
        sample_kind: ResourceSampleKind::Final,
        metadata,
        ..ResourcePayload::default()
    };
    if let Some(counters) = draft.counters {
        apply_cgroup_counters(&mut payload, counters);
    }
    payload
}

fn unix_seconds(time: SystemTime) -> Option<u64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .ok()
}

fn trace_requests_resource_metrics(entry: &TraceEntry) -> bool {
    entry.sensor_plan.collectors.iter().any(|collector| {
        collector
            .capabilities
            .iter()
            .any(|capability| *capability == Capability::ResourceMetrics)
    })
}

fn recovered_root_is_gone(
    identity: ProcessIdentity,
    process_registry: &ProcessIdentityManager,
) -> Result<bool, ControlError> {
    let Some(host) = process_registry
        .record(identity)
        .and_then(|record| record.host.as_ref())
    else {
        return Ok(true);
    };
    let Some(stat) = read_proc_stat(host.pid)? else {
        return Ok(true);
    };
    Ok(stat.exited
        || (host.start_time_ticks != 0 && stat.start_time_ticks != host.start_time_ticks))
}

fn sample_identities(entry: &TraceEntry, include_children: bool) -> Vec<ProcessIdentity> {
    if include_children {
        return entry
            .memberships
            .memberships()
            .filter(|membership| membership.capture_enabled)
            .filter(|membership| {
                matches!(
                    membership.state,
                    MembershipState::Starting | MembershipState::Active
                )
            })
            .map(|membership| membership.identity.clone())
            .collect();
    }
    vec![entry.trace.root_process_identity.clone()]
}

fn recovered_sample_identities(
    storage: &dyn storage_core::StorageBackend,
    trace_id: TraceId,
    root: ProcessIdentity,
    include_children: bool,
) -> Result<Vec<ProcessIdentity>, ControlError> {
    if !include_children {
        return Ok(vec![root]);
    }
    let mut identities = storage
        .trace_memberships(trace_id)
        .map_err(|error| ControlError::new(error.stage, error.message))?
        .into_iter()
        .filter(|membership| membership.capture_enabled)
        .filter(|membership| {
            matches!(
                membership.state,
                MembershipState::Starting | MembershipState::Active
            )
        })
        .map(|membership| membership.identity)
        .collect::<Vec<_>>();
    if !identities.contains(&root) {
        identities.push(root);
    }
    Ok(identities)
}

fn resource_alert(config: &ResourceMetricsConfig, cpu_percent_millis: u64, rss_kb: u64) -> bool {
    config
        .cpu_alert_percent_millis
        .map(|threshold| cpu_percent_millis >= threshold)
        .unwrap_or(false)
        || config
            .memory_alert_rss_kb
            .map(|threshold| rss_kb >= threshold)
            .unwrap_or(false)
}

fn add_system_metadata(metadata: &mut BTreeMap<String, String>, system: SystemMetrics) {
    metadata.insert(
        "host_mem_total_kb".to_string(),
        system.mem_total_kb.to_string(),
    );
    metadata.insert(
        "host_mem_free_kb".to_string(),
        system.mem_free_kb.to_string(),
    );
    metadata.insert(
        "host_mem_available_kb".to_string(),
        system.mem_available_kb.to_string(),
    );
    metadata.insert("host_loadavg_1m".to_string(), system.loadavg_1m);
    metadata.insert("host_loadavg_5m".to_string(), system.loadavg_5m);
    metadata.insert("host_loadavg_15m".to_string(), system.loadavg_15m);
    metadata.insert(
        "host_loadavg_running_threads".to_string(),
        system.loadavg_running_threads,
    );
    metadata.insert(
        "host_loadavg_total_threads".to_string(),
        system.loadavg_total_threads,
    );
    metadata.insert("host_loadavg_last_pid".to_string(), system.loadavg_last_pid);
}

fn format_percent_millis(value: u64) -> String {
    format!("{}.{:03}", value / 1000, value % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use linux_platform::cgroup_v2::{CgroupCounters, CpuStat, IoStat};
    use model_core::container::{ContainerRuntime, NormalizedContainerId};
    use model_core::external_cgroup::{ExternalBindingState, ExternalCgroupBinding, HostBootId};
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn binding() -> ExternalCgroupBinding {
        ExternalCgroupBinding::active(
            TraceId::new(7),
            ContainerRuntime::Docker,
            NormalizedContainerId::from_lower_hex(ID).unwrap(),
            PathBuf::from(format!("docker/{ID}")),
            0,
            1,
            HostBootId::from_bytes([1; 16]),
            UNIX_EPOCH + Duration::from_secs(100),
        )
    }

    fn counters() -> CgroupCounters {
        CgroupCounters {
            memory_current_bytes: 4096,
            memory_peak_bytes: Some(8192),
            memory_anon_bytes: Some(2048),
            memory_file_bytes: Some(1024),
            memory_swap_current_bytes: Some(0),
            memory_events: None,
            memory_events_local: None,
            cpu: CpuStat {
                usage_usec: 1000,
                user_usec: Some(500),
                system_usec: Some(400),
                nr_throttled: Some(1),
                throttled_usec: Some(2),
                unknown: BTreeMap::new(),
            },
            io: Some(IoStat {
                read_bytes: Some(10),
                write_bytes: Some(20),
                unknown: BTreeMap::new(),
            }),
            pids_current: Some(3),
            pids_peak: Some(5),
        }
    }

    #[test]
    fn final_payload_uses_container_scope_and_broader_coverage_with_counters() {
        let payload = build_external_final_payload(ExternalFinalDraft {
            binding: binding(),
            counters: Some(counters()),
            read_error: None,
        });
        assert_eq!(payload.scope, "container");
        assert_eq!(payload.subject, format!("container:{ID}"));
        assert_eq!(
            payload.accounting_method,
            ResourceAccountingMethod::CgroupV2
        );
        assert_eq!(
            payload.accounting_coverage,
            ResourceAccountingCoverage::BroaderThanTrace
        );
        assert_eq!(payload.sample_kind, ResourceSampleKind::Final);
        assert_eq!(payload.memory_current_bytes, Some(4096));
        assert_eq!(payload.memory_peak_bytes, Some(8192));
        assert_eq!(
            payload.metadata.get("ownership").map(String::as_str),
            Some("runtime-owned")
        );
        assert_eq!(
            payload.metadata.get("runtime").map(String::as_str),
            Some("docker")
        );
        assert_eq!(
            payload
                .metadata
                .get("memory_peak_window")
                .map(String::as_str),
            Some("cgroup_lifetime")
        );
        assert!(payload.metadata.contains_key("relative_path"));
    }

    #[test]
    fn final_payload_marks_stale_binding_partial() {
        let mut stale = binding();
        stale.lifecycle_state = ExternalBindingState::Stale;
        stale.stale_reason =
            Some(model_core::external_cgroup::ExternalBindingStaleReason::RepeatedReadFailure);
        stale.last_good_at = Some(UNIX_EPOCH + Duration::from_secs(50));
        let payload = build_external_final_payload(ExternalFinalDraft {
            binding: stale,
            counters: None,
            read_error: None,
        });
        assert_eq!(
            payload.accounting_coverage,
            ResourceAccountingCoverage::Partial
        );
        assert_eq!(payload.memory_current_bytes, None);
        assert_eq!(
            payload.metadata.get("fallback_from").map(String::as_str),
            Some("external_container_cgroup")
        );
        assert_eq!(
            payload.metadata.get("stale_reason").map(String::as_str),
            Some("repeated_read_failure")
        );
        assert!(payload.metadata.contains_key("last_good_at_unix_seconds"));
    }

    #[test]
    fn apply_counters_populates_typed_fields() {
        let mut payload = ResourcePayload::default();
        apply_cgroup_counters(&mut payload, counters());
        assert_eq!(payload.cpu_usage_usec, Some(1000));
        assert_eq!(payload.io_read_bytes, Some(10));
        assert_eq!(payload.pids_current, Some(3));
        assert_eq!(payload.memory_swap_current_bytes, Some(0));
    }

    #[test]
    fn cgroup_cpu_rate_needs_a_prior_sample() {
        let trace = TraceId::new(1);
        let empty: BTreeMap<TraceId, CgroupCpuSample> = BTreeMap::new();
        assert!(cgroup_cpu_rate(&empty, trace, 1000, Instant::now()).is_none());

        let now = Instant::now();
        let previous = BTreeMap::from([(
            trace,
            CgroupCpuSample {
                usage_usec: 0,
                sampled_at: now,
            },
        )]);
        let rate = cgroup_cpu_rate(&previous, trace, 1_000_000, now + Duration::from_secs(1))
            .expect("a prior sample should produce a rate");
        assert_eq!(rate.percent_millis, 100_000);
        assert_eq!(rate.interval_usec, 1_000_000);
    }
}

// Sampling and finalization of managed trace scopes on one hierarchy.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use super::super::sampler::CgroupSamplerFactory;
use super::{
    ManagedHierarchy, ManagedRecoveryRecord, ManagedScopeAdmission, ManagedScopeManager,
    ManagedScopePersistence, ResourceFinalizationDraft, ResourceFinalizationPoll,
    apply_cgroup_counters,
};
use crate::model::resource_scope::TraceResourceScope;
use crate::model::{ResourceAccountingCoverage, ResourceMetrics, ResourceSampleKind};
use crate::{
    ProfilingError, ResourceSampleDraft, ResourceSamplingFailure, SamplingContext,
    SamplingSelection, WorkloadBackendMode as ResourceMetricsMode, WorkloadTargets,
};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

pub(crate) struct ManagedScopeRuntime<H: ManagedHierarchy> {
    mode: ResourceMetricsMode,
    finalization_timeout_ms: u64,
    manager: ManagedScopeManager<H>,
    finalization_started_at: BTreeMap<TraceId, Instant>,
    finalized_barriers: BTreeSet<TraceId>,
    diagnosed_failures: BTreeSet<TraceId>,
}

impl<H: ManagedHierarchy> ManagedScopeRuntime<H> {
    /// Initializes one version using only records owned by that hierarchy.
    /// An unavailable hierarchy may be skipped only when it has no live scopes
    /// to recover.
    pub(crate) fn try_initialize(
        config: &H::Config,
        sampler: CgroupSamplerFactory,
        storage: &mut dyn ManagedScopePersistence,
        records: &[ManagedRecoveryRecord],
    ) -> Result<(Option<Self>, Option<String>), ProfilingError> {
        let recovery_required = records.iter().any(|record| {
            record.scope.lifecycle_state.is_live()
                && record.scope.accounting_method == H::ACCOUNTING_METHOD
        });
        let owned_records = records
            .iter()
            .filter(|record| record.scope.accounting_method == H::ACCOUNTING_METHOD)
            .cloned()
            .collect();
        let started: Result<Self, ProfilingError> = (|| {
            let hierarchy = H::prepare(config)?;
            let manager = ManagedScopeManager::initialize(
                hierarchy,
                H::recovery_scan_limit(config),
                sampler,
                storage,
                owned_records,
            )?;
            Ok(Self::new(
                manager,
                H::finalization_timeout_ms(config),
                ResourceMetricsMode::Auto,
            ))
        })();
        match started {
            Ok(runtime) => Ok((Some(runtime), None)),
            Err(error) if !recovery_required && error.is_unavailable() => {
                let reason = format!(
                    "{}: {}: {}",
                    H::ACCOUNTING_METHOD.as_str(),
                    error.code,
                    error.message
                );
                tracing::warn!(reason = %reason, "managed cgroup admission unavailable");
                Ok((None, Some(reason)))
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn new(
        mut manager: ManagedScopeManager<H>,
        finalization_timeout_ms: u64,
        mode: ResourceMetricsMode,
    ) -> Self {
        let now = Instant::now();
        let system_now = SystemTime::now();
        let (waiting_since, finalized_barriers) = manager.take_recovery_state();
        let finalization_started_at = waiting_since
            .into_iter()
            .map(|(id, started)| {
                let elapsed = system_now.duration_since(started).unwrap_or_default();
                (id, now.checked_sub(elapsed).unwrap_or(now))
            })
            .collect();
        Self {
            mode,
            finalization_timeout_ms,
            manager,
            finalization_started_at,
            finalized_barriers,
            diagnosed_failures: BTreeSet::new(),
        }
    }

    /// Admit a daemon-controlled, SIGSTOPped launch into its trace scope.
    ///
    /// Scope creation/controller setup may fall back in `auto` mode. Moving the
    /// PID is the commit boundary: any error at or after that point is fatal.
    pub(crate) fn admit_stopped_launch(
        &mut self,
        trace_id: TraceId,
        pid: u32,
    ) -> Result<ManagedScopeAdmission, ProfilingError> {
        self.manager.admit_stopped_launch(trace_id, pid, self.mode)
    }

    pub(crate) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ProfilingError> {
        self.manager.activate_scope(scope)
    }

    pub(crate) fn maintain(&mut self) {
        self.manager.cleanup_abandoned_admissions();
    }

    /// Samples every live trace with a managed scope, then every recovered
    /// scope whose trace is no longer tracked. Claims what it samples.
    pub(crate) fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: SamplingContext,
        selection: &mut SamplingSelection,
        drafts: &mut Vec<ResourceSampleDraft>,
        failures: &mut Vec<ResourceSamplingFailure>,
    ) {
        for trace in targets.traces() {
            if selection.is_claimed(trace.trace_id) || !trace.requests_resources {
                continue;
            }
            let Some(scope) = self.manager.scope(trace.trace_id).cloned() else {
                continue;
            };
            selection.claim(trace.trace_id);
            let outcome = self.collect_sample(trace.trace_id, trace.root, &scope, context, false);
            self.record_outcome(trace.trace_id, false, outcome, drafts, failures);
        }
        let recovered = self
            .manager
            .recovered_processes()
            .filter_map(|(trace_id, process)| {
                if targets.contains_trace(trace_id) {
                    return None;
                }
                self.manager
                    .scope(trace_id)
                    .cloned()
                    .map(|scope| (trace_id, process, scope))
            })
            .collect::<Vec<_>>();
        for (trace_id, process, scope) in recovered {
            if selection.is_claimed(trace_id) {
                continue;
            }
            selection.claim(trace_id);
            let outcome = self.collect_sample(trace_id, process, &scope, context, true);
            self.record_outcome(trace_id, true, outcome, drafts, failures);
        }
    }

    fn record_outcome(
        &mut self,
        trace_id: TraceId,
        recovered: bool,
        outcome: Result<ResourceSampleDraft, ProfilingError>,
        drafts: &mut Vec<ResourceSampleDraft>,
        failures: &mut Vec<ResourceSamplingFailure>,
    ) {
        match outcome {
            Ok(draft) => {
                self.diagnosed_failures.remove(&trace_id);
                drafts.push(draft);
            }
            Err(error) => {
                if self.diagnosed_failures.insert(trace_id) {
                    failures.push(ResourceSamplingFailure {
                        trace_id,
                        message: format!("{}: {}", error.code, error.message),
                        recovered,
                    });
                }
            }
        }
    }

    pub(crate) fn poll_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
    ) -> Result<ResourceFinalizationPoll, ProfilingError> {
        let mut candidates = targets
            .traces()
            .into_iter()
            .filter(|trace| trace.draining || trace.terminal)
            .filter_map(|trace| {
                self.manager
                    .scope(trace.trace_id)
                    .cloned()
                    .map(|scope| (trace.trace_id, trace.root, scope, false))
            })
            .collect::<Vec<_>>();
        for (trace_id, process) in self.manager.recovered_processes() {
            if candidates
                .iter()
                .any(|(candidate, _, _, _)| *candidate == trace_id)
            {
                continue;
            }
            let Some(scope) = self.manager.scope(trace_id).cloned() else {
                continue;
            };
            let already_waiting = self.finalization_started_at.contains_key(&trace_id);
            let empty = self
                .manager
                .populated(&scope)
                .map(|populated| !populated)
                .unwrap_or(false);
            let root_gone = match targets.root_is_gone(process) {
                Ok(gone) => gone,
                Err(error) => {
                    tracing::warn!(%trace_id,%error,"managed root liveness unavailable; retrying");
                    false
                }
            };
            if already_waiting || empty || root_gone {
                candidates.push((trace_id, process, scope, true));
            }
        }
        let now = Instant::now();
        let observed_at = SystemTime::now();
        let timeout = Duration::from_millis(self.finalization_timeout_ms);
        let mut poll = ResourceFinalizationPoll::default();

        for (trace_id, process, scope, recovered) in candidates {
            let started_at = *self.finalization_started_at.entry(trace_id).or_insert(now);
            poll.waiting.push(trace_id);
            let timed_out = now.saturating_duration_since(started_at) >= timeout;
            let mut finalization_timed_out = match self.manager.populated(&scope) {
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

            let context = SamplingContext {
                sampled_at: now,
                observed_at,
            };
            let mut sample =
                match self.collect_sample(trace_id, process, &scope, context, recovered) {
                    Ok(sample) => sample,
                    Err(error) if timed_out => {
                        finalization_timed_out = true;
                        ResourceSampleDraft {
                            trace_id,
                            observed_at,
                            process,
                            recovered,
                            payload: ResourceMetrics {
                                scope: "trace".to_string(),
                                subject: trace_id.to_string(),
                                accounting_method: H::ACCOUNTING_METHOD,
                                accounting_coverage: ResourceAccountingCoverage::Partial,
                                sample_kind: ResourceSampleKind::Final,
                                metadata: BTreeMap::from([
                                    ("alert".to_string(), "false".to_string()),
                                    ("finalization_timeout".to_string(), "true".to_string()),
                                    ("finalization_read_error".to_string(), error.message),
                                ]),
                                ..ResourceMetrics::default()
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
                    self.finalization_timeout_ms.to_string(),
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

    pub(crate) fn finish_finalization(&mut self, trace_id: TraceId, remove_scope: bool) {
        self.finalized_barriers.insert(trace_id);
        self.diagnosed_failures.remove(&trace_id);
        self.finalization_started_at.remove(&trace_id);
        self.manager.finish(trace_id, remove_scope);
    }

    pub(crate) fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        self.finalized_barriers
            .retain(|id| targets.contains_trace(*id));
    }

    pub(crate) fn resource_barrier_ready(&self, trace_id: TraceId) -> bool {
        !self.manager.has_scope(trace_id)
    }

    pub(crate) fn completed_managed_barrier(&self, trace_id: TraceId) -> bool {
        self.finalized_barriers.contains(&trace_id)
    }

    pub(crate) fn has_pending_barrier(&self, trace_id: TraceId) -> bool {
        self.manager.has_scope(trace_id)
    }

    pub(crate) fn force_pending_finalizations_due(&mut self) {
        let elapsed = Duration::from_millis(self.finalization_timeout_ms);
        let due = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
        for started_at in self.finalization_started_at.values_mut() {
            *started_at = due;
        }
    }

    fn collect_sample(
        &mut self,
        trace_id: TraceId,
        process: ProcessIdentity,
        scope: &linux_cgroup::TraceScopePaths,
        context: SamplingContext,
        recovered: bool,
    ) -> Result<ResourceSampleDraft, ProfilingError> {
        let sample = self
            .manager
            .sample(trace_id, process, context)
            .map_err(|error| ProfilingError::new("cgroup_sample", error))?;
        let counters = sample.counters;
        let cpu_percent_millis = sample.cpu_percent_millis;
        let process_rss = sample.process_rss;
        let mut metadata = BTreeMap::from([
            (
                "cgroup_relative_path".to_string(),
                scope.relative_path.display().to_string(),
            ),
            ("cpu_rate_basis".to_string(), H::CPU_RATE_BASIS.to_string()),
        ]);
        metadata.extend(
            H::SAMPLE_METADATA
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string())),
        );
        if let Some(interval) = sample.cpu_interval_usec {
            metadata.insert("cpu_rate_interval_usec".to_string(), interval.to_string());
        }
        if let Some(error) = sample.process_rss_error {
            metadata.insert("process_rss_error".into(), error);
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
        }
        for (key, value) in &counters.cpu.unknown {
            metadata.insert(format!("cpu.stat.{key}"), value.clone());
        }
        if let Some(io) = &counters.io {
            for (key, value) in &io.unknown {
                metadata.insert(format!("io.stat.{key}"), value.clone());
            }
        } else {
            metadata.insert(H::MISSING_IO_FILE_KEY.to_string(), "true".to_string());
        }
        if counters.pids_current.is_none() {
            metadata.insert(
                "missing_optional_file.pids.current".to_string(),
                "true".to_string(),
            );
        }
        let mut payload = ResourceMetrics {
            scope: "trace".to_string(),
            subject: trace_id.to_string(),
            accounting_method: H::ACCOUNTING_METHOD,
            accounting_coverage: ResourceAccountingCoverage::Exact,
            sample_kind: ResourceSampleKind::Periodic,
            cpu_percent_millis,
            rss_kb: process_rss.map(|sample| sample.rss_sum_kb),
            process_rss_sum_kb: process_rss.map(|sample| sample.rss_sum_kb),
            metadata,
            ..ResourceMetrics::default()
        };
        apply_cgroup_counters(&mut payload, counters);
        Ok(ResourceSampleDraft {
            trace_id,
            observed_at: context.observed_at,
            process,
            recovered,
            payload,
        })
    }
}

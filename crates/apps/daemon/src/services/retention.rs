//! Time-based storage retention for terminal traces.

use std::collections::BTreeSet;
use std::time::{Duration, Instant, SystemTime};

use config_core::daemon::StorageRetentionConfig;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::trace::{TraceLifecycleState, TraceRecord};
use storage_core::{StorageBackend, TraceFilter, TraceTombstone};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetentionSweepReport {
    pub purged_traces: usize,
    pub skipped_export_leases: usize,
}

pub(crate) struct StorageRetentionService {
    config: StorageRetentionConfig,
    next_sweep_at: Option<Instant>,
}

impl StorageRetentionService {
    pub(crate) fn new(config: StorageRetentionConfig) -> Self {
        Self {
            next_sweep_at: config.enabled.then(Instant::now),
            config,
        }
    }

    pub(crate) fn poll_timeout(&self) -> Option<Duration> {
        if !self.config.enabled {
            return None;
        }
        let next_sweep_at = self.next_sweep_at?;
        let now = Instant::now();
        Some(next_sweep_at.saturating_duration_since(now))
    }

    pub(crate) fn sweep_if_due(
        &mut self,
        storage: &mut dyn StorageBackend,
        trace_runtime: &mut TraceRuntime,
        finalized_terminal_traces: &mut BTreeSet<TraceId>,
        pending_terminal_finalizations: &mut BTreeSet<TraceId>,
    ) -> Result<Option<RetentionSweepReport>, ControlError> {
        if !self.config.enabled {
            return Ok(None);
        }
        let now_instant = Instant::now();
        if self
            .next_sweep_at
            .is_some_and(|next_sweep_at| next_sweep_at > now_instant)
        {
            return Ok(None);
        }
        self.next_sweep_at = Some(now_instant + self.config.sweep_interval);

        let now = SystemTime::now();
        let mut candidates =
            self.expired_candidates(storage, trace_runtime, finalized_terminal_traces, now)?;
        candidates.sort_by_key(|candidate| (candidate.finished_at, candidate.trace.trace_id));
        let max_traces = usize::try_from(self.config.max_traces_per_sweep).map_err(|error| {
            ControlError::new(
                "retention_config",
                format!("max_traces_per_sweep overflow: {error}"),
            )
        })?;

        let mut report = RetentionSweepReport {
            purged_traces: 0,
            skipped_export_leases: 0,
        };
        for candidate in candidates.into_iter().take(max_traces) {
            let trace_id = candidate.trace.trace_id;
            let reason = format!(
                "storage retention expired: max_trace_age={} finished_at_age={}",
                duration_label(self.config.max_trace_age),
                duration_label(candidate.age)
            );
            let tombstone = TraceTombstone {
                trace_id,
                lifecycle_state: candidate.trace.lifecycle_state,
                health: candidate.trace.health,
                cleaned_at: now,
                cleanup_reason: reason,
            };
            match storage.purge_trace(trace_id, tombstone) {
                Ok(()) => {
                    trace_runtime.forget_trace(trace_id);
                    finalized_terminal_traces.remove(&trace_id);
                    pending_terminal_finalizations.remove(&trace_id);
                    report.purged_traces += 1;
                    tracing::info!(%trace_id, "retention purged expired trace");
                }
                Err(error) if is_export_lease_error(&error) => {
                    report.skipped_export_leases += 1;
                    tracing::debug!(
                        %trace_id,
                        error.stage = %error.stage,
                        error.message = %error.message,
                        "retention skipped trace with active export lease"
                    );
                }
                Err(error) => {
                    return Err(ControlError::new(error.stage, error.message));
                }
            }
        }
        if report.purged_traces > 0 && self.config.checkpoint_after_sweep {
            storage
                .checkpoint()
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        Ok(Some(report))
    }

    fn expired_candidates(
        &self,
        storage: &dyn StorageBackend,
        trace_runtime: &TraceRuntime,
        finalized_terminal_traces: &BTreeSet<TraceId>,
        now: SystemTime,
    ) -> Result<Vec<ExpiredTraceCandidate>, ControlError> {
        let traces = storage
            .list_traces(&TraceFilter::default())
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let mut candidates = Vec::new();
        for trace in traces {
            if !trace.lifecycle_state.is_terminal() {
                continue;
            }
            if self.trace_is_protected(&trace) {
                continue;
            }
            if let Some(entry) = trace_runtime.get_trace(trace.trace_id)
                && (!entry.trace.lifecycle_state.is_terminal()
                    || !finalized_terminal_traces.contains(&trace.trace_id))
            {
                continue;
            }
            let Some(finished_at) = trace_finished_at(&trace) else {
                continue;
            };
            let Ok(age) = now.duration_since(finished_at) else {
                continue;
            };
            if age < self.config.max_trace_age || age < self.config.min_terminal_age {
                continue;
            }
            candidates.push(ExpiredTraceCandidate {
                trace,
                finished_at,
                age,
            });
        }
        Ok(candidates)
    }

    fn trace_is_protected(&self, trace: &TraceRecord) -> bool {
        self.config
            .protected_tags
            .iter()
            .any(|tag| trace.tags.contains(tag))
    }
}

struct ExpiredTraceCandidate {
    trace: TraceRecord,
    finished_at: SystemTime,
    age: Duration,
}

fn trace_finished_at(trace: &TraceRecord) -> Option<SystemTime> {
    match trace.lifecycle_state {
        TraceLifecycleState::Completed => trace.timings.completed_at,
        TraceLifecycleState::Exited => trace.timings.exited_at,
        TraceLifecycleState::Failed => trace.timings.failed_at,
        _ => None,
    }
    .or(Some(trace.timings.created_at))
}

fn is_export_lease_error(error: &storage_core::StorageError) -> bool {
    error.stage == "purge_trace" && error.message == "active export lease blocks purge"
}

fn duration_label(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if duration.subsec_nanos() == 0 {
        if seconds % (24 * 60 * 60) == 0 {
            return format!("{}d", seconds / (24 * 60 * 60));
        }
        if seconds % (60 * 60) == 0 {
            return format!("{}h", seconds / (60 * 60));
        }
        if seconds % 60 == 0 {
            return format!("{}m", seconds / 60);
        }
        return format!("{seconds}s");
    }
    format!("{}ms", duration.as_millis())
}

impl StorageAttachService {
    pub(in crate::services) fn sweep_storage_retention_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let Some(report) = self.storage_retention.sweep_if_due(
            self.storage.as_mut(),
            trace_runtime,
            &mut self.finalized_terminal_traces,
            &mut self.pending_terminal_finalizations,
        )?
        else {
            return Ok(());
        };
        if report.purged_traces > 0 || report.skipped_export_leases > 0 {
            tracing::info!(
                purged_traces = report.purged_traces,
                skipped_export_leases = report.skipped_export_leases,
                "retention sweep completed"
            );
        }
        Ok(())
    }
}

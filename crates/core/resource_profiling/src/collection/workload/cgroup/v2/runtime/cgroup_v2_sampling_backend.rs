pub(super) use std::collections::{BTreeMap, BTreeSet};
pub(super) use std::time::{Duration, Instant, SystemTime};

pub(super) use super::completion::*;
pub(super) use crate::collection::workload::profiler::workload_sample::*;
pub(super) use crate::model::MemoryEventCounters;
pub(super) use crate::model::ResourceAccountingCoverage;
pub(super) use crate::model::ResourceAccountingMethod;
pub(super) use crate::model::ResourceMetrics;
pub(super) use crate::model::ResourceSampleKind;
pub(super) use crate::model::resource_scope::TraceResourceScope;
pub(super) use crate::{CgroupV2Config, WorkloadBackendMode as ResourceMetricsMode};
pub(super) use crate::{CgroupV2Persistence, WorkloadTarget, WorkloadTargets};
pub(super) use crate::{ProfilingError, SamplingContext};
pub(super) use model_core::ids::TraceId;
pub(super) use model_core::process::{HostProcessCoordinates, ProcessIdentity};

pub(super) use crate::collection::workload::cgroup::v2::{CgroupAdmission, ManagedScopeManager};
pub(super) use crate::collection::workload::cgroup::v2::{
    ExternalAdmission, ExternalFinalDraft, ExternalRead, ExternalScopeManager,
};

pub struct CgroupV2SamplingBackend {
    pub(super) mode: ResourceMetricsMode,
    pub(super) finalization_timeout_ms: u64,
    pub(super) cgroup: Option<ManagedScopeManager>,
    pub(super) external: Option<ExternalScopeManager>,
    pub(super) cgroup_fallback_reason: Option<String>,
    pub(super) trace_fallback_reasons: BTreeMap<TraceId, String>,
    pub(super) finalization_started_at: BTreeMap<TraceId, Instant>,
    pub(super) finalized_barriers: BTreeSet<TraceId>,
    pub(super) diagnosed_cgroup_failures: BTreeSet<TraceId>,
    pub(super) diagnosed_external_failures: BTreeSet<TraceId>,
}
impl CgroupV2SamplingBackend {
    pub(crate) fn available(&self) -> bool {
        self.cgroup.is_some()
            || self
                .external
                .as_ref()
                .is_some_and(|runtime| runtime.is_available())
    }

    pub(crate) fn new(
        config: CgroupV2Config,
        mode: ResourceMetricsMode,
        storage: &mut dyn CgroupV2Persistence,
    ) -> Result<Self, ProfilingError> {
        let finalization_timeout_ms = config.managed.finalization_timeout_ms;
        let (mut cgroup, external, cgroup_fallback_reason) =
            super::super::CgroupV2BackendFactory::initialize(config, mode, storage)?;
        let now = Instant::now();
        let system_now = SystemTime::now();
        let (waiting_since, finalized_barriers) = cgroup
            .as_mut()
            .map(|runtime| runtime.take_recovery_state())
            .unwrap_or_default();
        let finalization_started_at = waiting_since
            .into_iter()
            .map(|(id, started)| {
                let elapsed = system_now.duration_since(started).unwrap_or_default();
                (id, now.checked_sub(elapsed).unwrap_or(now))
            })
            .collect();
        Ok(Self {
            mode,
            finalization_timeout_ms,
            cgroup,
            external,
            cgroup_fallback_reason,
            trace_fallback_reasons: BTreeMap::new(),
            finalization_started_at,
            finalized_barriers,
            diagnosed_cgroup_failures: BTreeSet::new(),
            diagnosed_external_failures: BTreeSet::new(),
        })
    }
}

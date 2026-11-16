pub(super) use std::collections::{BTreeMap, BTreeSet};
pub(super) use std::time::{Instant, SystemTime};

pub(super) use super::completion::*;
use crate::collection::workload::cgroup::common::managed::ManagedScopeRuntime;
pub(super) use crate::collection::workload::cgroup::common::managed::apply_cgroup_counters;
pub(super) use crate::collection::workload::cgroup::common::sampler::CgroupSamplerFactory;
use crate::collection::workload::cgroup::v2::ManagedV2Hierarchy;
pub(super) use crate::collection::workload::profiler::workload_sample::*;
pub(super) use crate::model::ResourceAccountingCoverage;
pub(super) use crate::model::ResourceAccountingMethod;
pub(super) use crate::model::ResourceMetrics;
pub(super) use crate::model::ResourceSampleKind;
pub(super) use crate::{CgroupV2BindingPersistence, WorkloadTarget, WorkloadTargets};
pub(super) use crate::{CgroupV2Config, WorkloadBackendMode as ResourceMetricsMode};
pub(super) use crate::{ProfilingError, SamplingContext};
pub(super) use model_core::ids::TraceId;
pub(super) use model_core::process::{HostProcessCoordinates, ProcessIdentity};

pub(super) use crate::collection::workload::cgroup::v2::{
    CgroupV2BindingAdmission, CgroupV2BindingFinalRead, CgroupV2BindingManager, CgroupV2BindingRead,
};

/// Cgroup v2 accounting: managed trace scopes and runtime-owned containers.
pub struct CgroupV2SamplingBackend {
    pub(super) managed: Option<ManagedScopeRuntime<ManagedV2Hierarchy>>,
    pub(super) mode: ResourceMetricsMode,
    pub(super) bindings: Option<CgroupV2BindingManager>,
    pub(super) trace_fallback_reasons: BTreeMap<TraceId, String>,
    pub(super) diagnosed_binding_failures: BTreeSet<TraceId>,
}
impl CgroupV2SamplingBackend {
    pub(crate) fn available(&self) -> bool {
        self.bindings
            .as_ref()
            .is_some_and(|runtime| runtime.is_available())
    }

    pub(crate) fn new(
        config: CgroupV2Config,
        sampler: CgroupSamplerFactory,
        mode: ResourceMetricsMode,
        storage: &mut dyn CgroupV2BindingPersistence,
        managed: Option<ManagedScopeRuntime<ManagedV2Hierarchy>>,
    ) -> Result<Self, ProfilingError> {
        let bindings = CgroupV2BindingManager::initialize(&config.external, sampler, storage)?;
        Ok(Self {
            managed,
            mode,
            bindings,
            trace_fallback_reasons: BTreeMap::new(),
            diagnosed_binding_failures: BTreeSet::new(),
        })
    }

    /// Why an attached container trace is not accounted by its runtime cgroup.
    pub(crate) fn fallback_reason(&self, trace_id: TraceId) -> Option<&str> {
        self.trace_fallback_reasons
            .get(&trace_id)
            .map(String::as_str)
    }
}

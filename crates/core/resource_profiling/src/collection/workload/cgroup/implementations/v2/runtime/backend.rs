use super::cgroup_v2_sampling_backend::*;
use crate::collection::workload::cgroup::cgroup_backend::CgroupBackend;
use crate::collection::workload::cgroup::common::managed::ManagedScopeRuntime;
use crate::collection::workload::cgroup::v2::ManagedV2Hierarchy;
use crate::{CgroupPersistence, ResourceMetricsDrain, SamplingSelection, WorkloadSamplingBackend};

impl CgroupBackend for CgroupV2SamplingBackend {
    type Hierarchy = ManagedV2Hierarchy;

    fn managed(&self) -> Option<&ManagedScopeRuntime<Self::Hierarchy>> {
        self.managed.as_ref()
    }
    fn managed_mut(&mut self) -> Option<&mut ManagedScopeRuntime<Self::Hierarchy>> {
        self.managed.as_mut()
    }
    fn available(&self) -> bool {
        CgroupV2SamplingBackend::available(self)
    }
    fn admit_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<bool, ProfilingError> {
        self.admit_container_binding(target, coordinates)?;
        Ok(self.v2_binding(target.trace_id).is_some())
    }
    fn collect_bindings(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        self.collect(targets, storage, context, selection)
    }
    fn prune_bindings(&mut self, targets: &dyn WorkloadTargets) {
        CgroupV2SamplingBackend::prune_forgotten(self, targets);
    }
    fn fallback_reason(&self, id: TraceId) -> Option<&str> {
        CgroupV2SamplingBackend::fallback_reason(self, id)
    }
}

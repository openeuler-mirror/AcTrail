use super::{CgroupV1BindingManager, ManagedV1Hierarchy};
use crate::collection::workload::cgroup::cgroup_backend::CgroupBackend;
use crate::collection::workload::cgroup::common::managed::ManagedScopeRuntime;
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use crate::{
    CgroupPersistence, ProfilingError, ResourceMetricsDrain, ResourceSampleDraft, SamplingContext,
    SamplingSelection, WorkloadTarget, WorkloadTargets,
};
use model_core::ids::TraceId;
use model_core::process::HostProcessCoordinates;

/// All cgroup v1 accounting: managed trace scopes and bindings to an existing
/// process cgroup (which may also belong to a container).
pub(crate) struct CgroupV1SamplingBackend {
    managed: Option<ManagedScopeRuntime<ManagedV1Hierarchy>>,
    bindings: CgroupV1BindingManager,
}

impl CgroupV1SamplingBackend {
    pub(crate) fn new(
        managed: Option<ManagedScopeRuntime<ManagedV1Hierarchy>>,
        bindings: CgroupV1BindingManager,
    ) -> Self {
        Self { managed, bindings }
    }
    pub(crate) fn binding(&self, id: TraceId) -> Option<CgroupV1Binding> {
        self.bindings.binding(id)
    }
    pub(crate) fn activate_v1_binding(&mut self, id: TraceId) {
        self.bindings.activate(id);
    }
    pub(crate) fn finish_v1_binding_finalization(&mut self, id: TraceId) {
        self.bindings.finish(id);
    }
    pub(crate) fn poll_v1_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
    ) -> Vec<ResourceSampleDraft> {
        self.bindings.poll_finalizations(targets, storage)
    }
    pub(crate) fn v1_binding_pending(&self, id: TraceId) -> bool {
        self.bindings.pending(id)
    }
    pub(crate) fn has_pending_v1_binding_finalizations(&self) -> bool {
        self.bindings.has_pending_finalizations()
    }
}

impl CgroupBackend for CgroupV1SamplingBackend {
    type Hierarchy = ManagedV1Hierarchy;

    fn managed(&self) -> Option<&ManagedScopeRuntime<Self::Hierarchy>> {
        self.managed.as_ref()
    }
    fn managed_mut(&mut self) -> Option<&mut ManagedScopeRuntime<Self::Hierarchy>> {
        self.managed.as_mut()
    }
    fn available(&self) -> bool {
        self.bindings.available()
    }
    fn admit_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<bool, ProfilingError> {
        self.bindings.admit(target, coordinates)?;
        Ok(true)
    }
    fn collect_bindings(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        self.bindings.collect(targets, storage, context, selection)
    }
}

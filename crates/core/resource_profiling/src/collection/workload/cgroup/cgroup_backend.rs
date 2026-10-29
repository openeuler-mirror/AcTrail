//! The shared version backend interface and its private static dispatcher.

use super::common::managed::{
    ManagedHierarchy, ManagedScopeAdmission, ManagedScopeRuntime, ResourceFinalizationPoll,
};
use super::{CgroupPersistence, v1::CgroupV1SamplingBackend, v2::CgroupV2SamplingBackend};
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use crate::model::cgroup_v2_binding::CgroupV2Binding;
use crate::model::resource_scope::TraceResourceScope;
use crate::{
    ProfilingError, ResourceMetricsDrain, SamplingContext, SamplingSelection, WorkloadTarget,
    WorkloadTargets,
};
use model_core::ids::TraceId;
use model_core::process::HostProcessCoordinates;

/// Operations common to version backends. Managed lifecycle behavior is shared
/// through `ManagedScopeRuntime`; each version owns its own runtime instance.
pub(crate) trait CgroupBackend {
    type Hierarchy: ManagedHierarchy;

    fn managed(&self) -> Option<&ManagedScopeRuntime<Self::Hierarchy>>;
    fn managed_mut(&mut self) -> Option<&mut ManagedScopeRuntime<Self::Hierarchy>>;
    fn available(&self) -> bool;
    fn admit_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<bool, ProfilingError>;
    fn collect_bindings(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError>;
    fn maintain_bindings(&mut self) {}
    fn prune_bindings(&mut self, _targets: &dyn WorkloadTargets) {}
    fn fallback_reason(&self, _id: TraceId) -> Option<&str> {
        None
    }

    fn has_managed(&self) -> bool {
        self.managed().is_some()
    }
    fn admit_managed_launch(
        &mut self,
        id: TraceId,
        pid: u32,
    ) -> Result<Option<ManagedScopeAdmission>, ProfilingError> {
        self.managed_mut()
            .map(|managed| managed.admit_stopped_launch(id, pid))
            .transpose()
    }
    fn activate_scope(&mut self, scope: &TraceResourceScope) -> Result<bool, ProfilingError> {
        let Some(managed) = self.managed_mut() else {
            return Ok(false);
        };
        managed.activate_scope(scope)?;
        Ok(true)
    }
    fn maintain(&mut self) {
        if let Some(managed) = self.managed_mut() {
            managed.maintain();
        }
        self.maintain_bindings();
    }
    fn collect_managed(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: SamplingContext,
        selection: &mut SamplingSelection,
        drain: &mut ResourceMetricsDrain,
    ) {
        if let Some(managed) = self.managed_mut() {
            managed.collect(
                targets,
                context,
                selection,
                &mut drain.samples,
                &mut drain.failures,
            );
        }
    }
    fn poll_managed_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
    ) -> Result<ResourceFinalizationPoll, ProfilingError> {
        match self.managed_mut() {
            Some(managed) => managed.poll_finalizations(targets),
            None => Ok(ResourceFinalizationPoll::default()),
        }
    }
    fn finish_managed_finalization(&mut self, id: TraceId, remove_scope: bool) {
        if let Some(managed) = self.managed_mut() {
            managed.finish_finalization(id, remove_scope);
        }
    }
    fn force_pending_finalizations_due(&mut self) {
        if let Some(managed) = self.managed_mut() {
            managed.force_pending_finalizations_due();
        }
    }
    fn managed_barrier_ready(&self, id: TraceId) -> bool {
        self.managed()
            .is_none_or(|managed| managed.resource_barrier_ready(id))
    }
    fn completed_managed_barrier(&self, id: TraceId) -> bool {
        self.managed()
            .is_some_and(|managed| managed.completed_managed_barrier(id))
    }
    fn has_pending_managed_barrier(&self, id: TraceId) -> bool {
        self.managed()
            .is_some_and(|managed| managed.has_pending_barrier(id))
    }
    fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        if let Some(managed) = self.managed_mut() {
            managed.prune_forgotten(targets);
        }
        self.prune_bindings(targets);
    }
}

/// Both variants are concrete: dispatch uses an enum, not a trait object.
pub(crate) enum VersionBackend {
    V2(CgroupV2SamplingBackend),
    V1(CgroupV1SamplingBackend),
}

macro_rules! dispatch {
    ($self:expr, $backend:ident => $call:expr) => {
        match $self {
            VersionBackend::V2($backend) => $call,
            VersionBackend::V1($backend) => $call,
        }
    };
}

impl VersionBackend {
    pub(crate) fn is_v2(&self) -> bool {
        matches!(self, Self::V2(_))
    }
    pub(crate) fn available(&self) -> bool {
        dispatch!(self, backend => backend.available())
    }
    pub(crate) fn has_managed(&self) -> bool {
        dispatch!(self, backend => backend.has_managed())
    }
    pub(crate) fn admit_managed_launch(
        &mut self,
        id: TraceId,
        pid: u32,
    ) -> Result<Option<ManagedScopeAdmission>, ProfilingError> {
        dispatch!(self, backend => backend.admit_managed_launch(id, pid))
    }
    pub(crate) fn admit_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<bool, ProfilingError> {
        dispatch!(self, backend => backend.admit_binding(target, coordinates))
    }
    pub(crate) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<bool, ProfilingError> {
        dispatch!(self, backend => backend.activate_scope(scope))
    }
    pub(crate) fn maintain(&mut self) {
        dispatch!(self, backend => backend.maintain())
    }
    pub(crate) fn collect_managed(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: SamplingContext,
        selection: &mut SamplingSelection,
        drain: &mut ResourceMetricsDrain,
    ) {
        dispatch!(self, backend => backend.collect_managed(targets, context, selection, drain))
    }
    pub(crate) fn collect_bindings(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        dispatch!(self, backend => backend.collect_bindings(targets, storage, context, selection))
    }
    pub(crate) fn poll_managed_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
    ) -> Result<ResourceFinalizationPoll, ProfilingError> {
        dispatch!(self, backend => backend.poll_managed_finalizations(targets))
    }
    pub(crate) fn finish_managed_finalization(&mut self, id: TraceId, remove_scope: bool) {
        dispatch!(self, backend => backend.finish_managed_finalization(id, remove_scope))
    }
    pub(crate) fn force_pending_finalizations_due(&mut self) {
        dispatch!(self, backend => backend.force_pending_finalizations_due())
    }
    pub(crate) fn managed_barrier_ready(&self, id: TraceId) -> bool {
        dispatch!(self, backend => backend.managed_barrier_ready(id))
    }
    pub(crate) fn completed_managed_barrier(&self, id: TraceId) -> bool {
        dispatch!(self, backend => backend.completed_managed_barrier(id))
    }
    pub(crate) fn has_pending_managed_barrier(&self, id: TraceId) -> bool {
        dispatch!(self, backend => backend.has_pending_managed_barrier(id))
    }
    pub(crate) fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        dispatch!(self, backend => backend.prune_forgotten(targets))
    }
    pub(crate) fn fallback_reason(&self, id: TraceId) -> Option<&str> {
        dispatch!(self, backend => backend.fallback_reason(id))
    }

    // These operations are intentionally version-specific and private to the
    // cgroup module. They do not widen the common backend interface.
    pub(crate) fn v2_binding(&self, id: TraceId) -> Option<CgroupV2Binding> {
        match self {
            Self::V2(backend) => backend.v2_binding(id),
            Self::V1(_) => None,
        }
    }
    pub(crate) fn poll_v2_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
    ) -> Result<Vec<super::CgroupV2BindingFinalization>, ProfilingError> {
        match self {
            Self::V2(backend) => backend.poll_v2_binding_finalizations(targets, storage),
            Self::V1(_) => Ok(Vec::new()),
        }
    }
    pub(crate) fn finish_v2_binding_finalization(&mut self, id: TraceId) {
        if let Self::V2(backend) = self {
            backend.finish_v2_binding_finalization(id);
        }
    }
    pub(crate) fn v2_binding_barrier_ready(&self, id: TraceId) -> bool {
        match self {
            Self::V2(backend) => backend.v2_binding_barrier_ready(id),
            Self::V1(_) => true,
        }
    }
    pub(crate) fn v1_binding(&self, id: TraceId) -> Option<CgroupV1Binding> {
        match self {
            Self::V1(backend) => backend.binding(id),
            Self::V2(_) => None,
        }
    }
    pub(crate) fn activate_v1_binding(&mut self, id: TraceId) {
        if let Self::V1(backend) = self {
            backend.activate_v1_binding(id);
        }
    }
    pub(crate) fn finish_v1_binding_finalization(&mut self, id: TraceId) {
        if let Self::V1(backend) = self {
            backend.finish_v1_binding_finalization(id);
        }
    }
    pub(crate) fn poll_v1_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
    ) -> Vec<crate::ResourceSampleDraft> {
        match self {
            Self::V1(backend) => backend.poll_v1_binding_finalizations(targets, storage),
            Self::V2(_) => Vec::new(),
        }
    }
    pub(crate) fn v1_binding_pending(&self, id: TraceId) -> bool {
        match self {
            Self::V1(backend) => backend.v1_binding_pending(id),
            Self::V2(_) => false,
        }
    }
    pub(crate) fn has_pending_v1_binding_finalizations(&self) -> bool {
        match self {
            Self::V1(backend) => backend.has_pending_v1_binding_finalizations(),
            Self::V2(_) => false,
        }
    }
}

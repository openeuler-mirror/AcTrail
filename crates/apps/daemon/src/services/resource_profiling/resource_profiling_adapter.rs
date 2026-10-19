use super::{
    scope_persistence_adapter::ScopePersistenceAdapter,
    workload_target_adapter::WorkloadTargetAdapter,
};
use control_contract::reply::ControlError;
use model_core::{ids::TraceId, process::HostProcessCoordinates};
use process_identity::ProcessIdentityManager;
use resource_profiling::model::{
    cgroup_v1_binding::CgroupV1Binding, external_cgroup::ExternalCgroupBinding,
    resource_scope::TraceResourceScope,
};
use resource_profiling::{
    ProfilingError, ResourceCompletionPoll, ResourceMetricsDrain, ResourceProfilingConfig,
    ResourceProfilingFactory, ResourceProfilingService,
};
use storage_core::StorageBackend;
use trace_runtime::{TraceRuntime, registry::TraceEntry};
pub(in crate::services) const COLLECTOR_NAME: &str = "resource-sampler";
pub(in crate::services) struct ResourceProfilingAdapter {
    inner: ResourceProfilingService,
}
impl ResourceProfilingAdapter {
    pub(in crate::services) fn new(
        config: ResourceProfilingConfig,
        storage: &mut dyn StorageBackend,
    ) -> Result<Self, ControlError> {
        Ok(Self {
            inner: ResourceProfilingFactory::build(
                config,
                &mut ScopePersistenceAdapter { storage },
            )
            .map_err(Self::error)?,
        })
    }
    fn error(error: ProfilingError) -> ControlError {
        ControlError::new(error.code, error.message)
    }
    fn coordinates(
        coordinates: &HostProcessCoordinates,
    ) -> Result<HostProcessCoordinates, ControlError> {
        let mut result = coordinates.clone();
        result.start_time_ticks = ebpf_collector::procfs::ProcfsIdentityReader
            .expected_start_time_ticks(coordinates)
            .ok_or_else(|| {
                ControlError::new(
                    "resource_identity",
                    "expected process generation unavailable",
                )
            })?;
        Ok(result)
    }
    pub(in crate::services) fn admit_stopped_launch(
        &mut self,
        entry: &TraceEntry,
        coordinates: &HostProcessCoordinates,
        launch: bool,
    ) -> Result<Option<TraceResourceScope>, ControlError> {
        self.inner
            .admit_stopped_launch(
                &WorkloadTargetAdapter::target(entry),
                &Self::coordinates(coordinates)?,
                launch,
            )
            .map_err(Self::error)
    }
    pub(in crate::services) fn admit_external_container(
        &mut self,
        entry: &TraceEntry,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ControlError> {
        self.inner
            .admit_external_container(
                &WorkloadTargetAdapter::target(entry),
                &Self::coordinates(coordinates)?,
            )
            .map_err(Self::error)
    }
    pub(in crate::services) fn activate_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ControlError> {
        self.inner.activate_scope(scope).map_err(Self::error)
    }
    pub(in crate::services) fn drain_due(
        &mut self,
        traces: &TraceRuntime,
        identities: &ProcessIdentityManager,
        storage: &mut dyn StorageBackend,
    ) -> Result<ResourceMetricsDrain, ControlError> {
        self.inner
            .drain_due(
                &WorkloadTargetAdapter::new(traces, Some(identities)),
                &mut ScopePersistenceAdapter { storage },
            )
            .map_err(Self::error)
    }
    pub(in crate::services) fn poll_finalizations(
        &mut self,
        traces: &TraceRuntime,
        identities: &ProcessIdentityManager,
    ) -> Result<ResourceCompletionPoll, ControlError> {
        self.inner
            .poll_finalizations(&WorkloadTargetAdapter::new(traces, Some(identities)))
            .map_err(Self::error)
    }
    pub(in crate::services) fn poll_external_finalizations(
        &mut self,
        traces: &TraceRuntime,
        identities: &ProcessIdentityManager,
        storage: &mut dyn StorageBackend,
    ) -> Result<Vec<TraceId>, ControlError> {
        self.inner
            .poll_external_finalizations(
                &WorkloadTargetAdapter::new(traces, Some(identities)),
                &mut ScopePersistenceAdapter { storage },
            )
            .map_err(Self::error)
    }
    pub(in crate::services) fn poll_v1_finalizations(
        &mut self,
        traces: &TraceRuntime,
        identities: &ProcessIdentityManager,
        storage: &mut dyn StorageBackend,
    ) -> Vec<TraceId> {
        self.inner.poll_v1_finalizations(
            &WorkloadTargetAdapter::new(traces, Some(identities)),
            &mut ScopePersistenceAdapter { storage },
        )
    }
    pub(in crate::services) fn prune_forgotten(&mut self, traces: &TraceRuntime) {
        self.inner
            .prune_forgotten(&WorkloadTargetAdapter::new(traces, None));
    }
    pub(in crate::services) fn finish_finalization(&mut self, id: TraceId, remove: bool) {
        self.inner.finish_finalization(id, remove)
    }
    pub(in crate::services) fn finish_external_finalization(&mut self, id: TraceId) {
        self.inner.finish_external_finalization(id)
    }
    pub(in crate::services) fn external_binding(
        &self,
        id: TraceId,
    ) -> Option<ExternalCgroupBinding> {
        self.inner.external_binding(id)
    }
    pub(in crate::services) fn resource_barrier_ready(&self, id: TraceId) -> bool {
        self.inner.resource_barrier_ready(id)
    }
    pub(in crate::services) fn completed_managed_barrier(&self, id: TraceId) -> bool {
        self.inner.completed_managed_barrier(id)
    }
    pub(in crate::services) fn has_pending_barrier(&self, id: TraceId) -> bool {
        self.inner.has_pending_barrier(id)
    }
    pub(in crate::services) fn external_barrier_ready(&self, id: TraceId) -> bool {
        self.inner.external_barrier_ready(id)
    }
    pub(in crate::services) fn force_pending_finalizations_due(&mut self) {
        self.inner.force_pending_finalizations_due()
    }
    pub(in crate::services) fn poll_timeout(&self) -> Option<std::time::Duration> {
        self.inner.poll_timeout()
    }
    pub(in crate::services) fn has_pending_v1_finalizations(&self) -> bool {
        self.inner.has_pending_v1_finalizations()
    }
    pub(in crate::services) fn v1_binding(&self, id: TraceId) -> Option<CgroupV1Binding> {
        self.inner.v1_binding(id)
    }
    pub(in crate::services) fn activate_v1(&mut self, id: TraceId) {
        self.inner.activate_v1(id)
    }
    pub(in crate::services) fn finish_v1_finalization(&mut self, id: TraceId) {
        self.inner.finish_v1_finalization(id)
    }
    pub(in crate::services) fn drain_host_due(&mut self) {
        self.inner.drain_host_due()
    }
}

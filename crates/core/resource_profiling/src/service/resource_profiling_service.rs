use super::resource_sample_sink::ResourceSampleSink;
use super::{ResourceCompletion, ResourceCompletionPoll};
use crate::model::{
    cgroup_v1_binding::CgroupV1Binding, cgroup_v2_binding::CgroupV2Binding,
    resource_scope::TraceResourceScope,
};
use crate::{
    CgroupPersistence, HostProfiler, ProfilingError, ResourceMetricsDrain, ResourceSampleDraft,
    WorkloadBackend, WorkloadProfiler, WorkloadTarget, WorkloadTargets,
};
use model_core::{ids::TraceId, process::HostProcessCoordinates};
use std::time::Duration;
pub struct ResourceProfilingService {
    pub(super) profiler: WorkloadProfiler<WorkloadBackend>,
    pub(super) host: Option<HostProfiler>,
    pub(super) sink: ResourceSampleSink,
}
impl ResourceProfilingService {
    pub fn admit_stopped_launch(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
        launch: bool,
    ) -> Result<Option<TraceResourceScope>, ProfilingError> {
        let Some(backend) = self.profiler.backend_mut().cgroup_mut() else {
            return Ok(None);
        };
        backend.admit_stopped_launch(target, coordinates, launch)
    }
    pub fn admit_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        let Some(backend) = self.profiler.backend_mut().cgroup_mut() else {
            return Ok(());
        };
        backend.admit_binding(target, coordinates)
    }
    pub fn activate_scope(&mut self, scope: &TraceResourceScope) -> Result<(), ProfilingError> {
        self.profiler
            .backend_mut()
            .cgroup_mut()
            .ok_or_else(|| {
                ProfilingError::new(
                    "resource_scope_activation",
                    "selected backend has no cgroup scope",
                )
            })?
            .activate_scope(scope)
    }

    pub fn drain_due(
        &mut self,
        targets: &dyn WorkloadTargets,
        persistence: &mut dyn CgroupPersistence,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let mut drain = self.profiler.drain_due(targets, persistence)?;
        self.sink.submit_drafts(std::mem::take(&mut drain.samples));
        Ok(drain)
    }

    pub fn poll_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
    ) -> Result<ResourceCompletionPoll, ProfilingError> {
        let Some(backend) = self.profiler.backend_mut().cgroup_mut() else {
            return Ok(ResourceCompletionPoll::default());
        };
        let poll = backend.poll_finalizations(targets)?;
        let mut ready = Vec::with_capacity(poll.ready.len());
        for finalization in poll.ready {
            self.sink.submit_drafts(vec![finalization.sample]);
            ready.push(ResourceCompletion {
                trace_id: finalization.trace_id,
                timed_out: finalization.timed_out,
                recovered: finalization.recovered,
            });
        }
        Ok(ResourceCompletionPoll {
            waiting: poll.waiting,
            ready,
        })
    }

    pub fn poll_v2_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        persistence: &mut dyn CgroupPersistence,
    ) -> Result<Vec<TraceId>, ProfilingError> {
        let Some(backend) = self.profiler.backend_mut().cgroup_mut() else {
            return Ok(Vec::new());
        };
        let drafts = backend.poll_v2_binding_finalizations(targets, persistence)?;
        let mut completed = Vec::with_capacity(drafts.len());
        for draft in drafts {
            completed.push(draft.trace_id);
            self.sink.submit_drafts(vec![ResourceSampleDraft {
                trace_id: draft.trace_id,
                process: draft.process,
                observed_at: draft.observed_at,
                payload: draft.payload,
                recovered: false,
            }]);
        }
        Ok(completed)
    }

    pub fn finish_finalization(&mut self, id: TraceId, remove: bool) {
        if let Some(backend) = self.profiler.backend_mut().cgroup_mut() {
            backend.finish_finalization(id, remove)
        }
    }
    pub fn finish_v2_binding_finalization(&mut self, id: TraceId) {
        if let Some(backend) = self.profiler.backend_mut().cgroup_mut() {
            backend.finish_v2_binding_finalization(id)
        }
    }
    pub fn v2_binding(&self, id: TraceId) -> Option<CgroupV2Binding> {
        self.profiler.backend().cgroup()?.v2_binding(id)
    }
    pub fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        if let Some(backend) = self.profiler.backend_mut().cgroup_mut() {
            backend.prune_forgotten(targets)
        }
    }
    pub fn resource_barrier_ready(&self, id: TraceId) -> bool {
        self.profiler
            .backend()
            .cgroup()
            .is_none_or(|backend| backend.resource_barrier_ready(id))
    }
    pub fn completed_managed_barrier(&self, id: TraceId) -> bool {
        self.profiler
            .backend()
            .cgroup()
            .is_some_and(|backend| backend.completed_managed_barrier(id))
    }
    pub fn has_pending_barrier(&self, id: TraceId) -> bool {
        self.profiler
            .backend()
            .cgroup()
            .is_some_and(|backend| backend.has_pending_barrier(id))
    }
    pub fn binding_barrier_ready(&self, id: TraceId) -> bool {
        self.profiler
            .backend()
            .cgroup()
            .is_none_or(|backend| backend.binding_barrier_ready(id))
    }
    pub fn force_pending_finalizations_due(&mut self) {
        if let Some(backend) = self.profiler.backend_mut().cgroup_mut() {
            backend.force_pending_finalizations_due()
        }
    }

    pub fn poll_timeout(&self) -> Option<Duration> {
        let host = self
            .host
            .as_ref()
            .map(|p| p.poll_timeout(std::time::Instant::now()));
        match (host, self.profiler.poll_timeout()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    pub fn drain_host_due(&mut self) {
        let Some(profiler) = self.host.as_mut() else {
            return;
        };
        match profiler.sample_due(crate::SamplingContext {
            sampled_at: std::time::Instant::now(),
            observed_at: std::time::SystemTime::now(),
        }) {
            Ok(Some(sample)) => self
                .sink
                .submit(vec![crate::model::ResourceSample::Host(sample)]),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error,"host resource sampling failed"),
        }
    }
}

impl ResourceProfilingService {
    pub fn has_pending_v1_binding_finalizations(&self) -> bool {
        self.profiler
            .backend()
            .cgroup()
            .is_some_and(|backend| backend.has_pending_v1_binding_finalizations())
    }
    pub fn v1_binding(&self, id: TraceId) -> Option<CgroupV1Binding> {
        self.profiler.backend().cgroup()?.v1_binding(id)
    }
    pub fn activate_v1_binding(&mut self, id: TraceId) {
        if let Some(backend) = self.profiler.backend_mut().cgroup_mut() {
            backend.activate_v1_binding(id);
        }
    }
    pub fn finish_v1_binding_finalization(&mut self, id: TraceId) {
        if let Some(backend) = self.profiler.backend_mut().cgroup_mut() {
            backend.finish_v1_binding_finalization(id);
        }
    }

    pub fn poll_v1_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        persistence: &mut dyn CgroupPersistence,
    ) -> Vec<TraceId> {
        let Some(backend) = self.profiler.backend_mut().cgroup_mut() else {
            return Vec::new();
        };
        let drafts = backend.poll_v1_binding_finalizations(targets, persistence);
        let completed = drafts.iter().map(|d| d.trace_id).collect();
        self.sink.submit_drafts(drafts);
        completed
    }
}

use super::{CgroupPersistence, v1::CgroupV1Runtime, v2::CgroupV2SamplingBackend};
use crate::model::resource_scope::TraceResourceScope;
use crate::{
    ProfilingError, ResourceMetricsDrain, SamplingContext, SamplingSelection, WorkloadBackendMode,
    WorkloadSamplingBackend, WorkloadTarget, WorkloadTargets,
};
use model_core::ids::TraceId;
use model_core::process::HostProcessCoordinates;
pub struct CgroupSamplingBackend {
    pub(super) v1: Option<CgroupV1Runtime>,
    pub(super) v2: Option<CgroupV2SamplingBackend>,
    pub(super) mode: WorkloadBackendMode,
}
impl CgroupSamplingBackend {
    pub fn v2(&self) -> Option<&CgroupV2SamplingBackend> {
        self.v2.as_ref()
    }
    pub fn v2_mut(&mut self) -> Option<&mut CgroupV2SamplingBackend> {
        self.v2.as_mut()
    }
    pub fn v1(&self) -> Option<&CgroupV1Runtime> {
        self.v1.as_ref()
    }
    pub fn v1_mut(&mut self) -> Option<&mut CgroupV1Runtime> {
        self.v1.as_mut()
    }
    pub fn admit_stopped_launch(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
        launch: bool,
    ) -> Result<Option<TraceResourceScope>, ProfilingError> {
        if !launch || !target.requests_resources {
            return Ok(None);
        }
        if let Some(v2) = self.v2.as_mut() {
            if let Some(scope) = v2.admit_stopped_launch(target, coordinates.pid, true)? {
                return Ok(Some(scope));
            }
        }
        self.admit_v1(target, coordinates)?;
        Ok(None)
    }
    pub fn admit_existing(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        if !target.requests_resources {
            return Ok(());
        }
        if let Some(v2) = self.v2.as_mut() {
            v2.admit_external_container(target, coordinates)?;
            if v2.external_binding(target.trace_id).is_some() {
                return Ok(());
            }
        }
        self.admit_v1(target, coordinates)
    }
    fn admit_v1(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        let reason = if let Some(v1) = self.v1.as_mut() {
            match v1.admit(target, coordinates) {
                Ok(()) => return Ok(()),
                Err(error) if error.is_unavailable() => error.message,
                Err(error) => return Err(error),
            }
        } else {
            "no eligible cgroup backend for target".into()
        };
        if self.mode == WorkloadBackendMode::Cgroup {
            return Err(ProfilingError::new("cgroup_admission", reason));
        }
        tracing::debug!(trace_id=%target.trace_id,%reason,"cgroup admission unavailable; outer auto may select procfs");
        Ok(())
    }
    pub fn external_barrier_ready(&self, id: TraceId) -> bool {
        self.v1.as_ref().is_none_or(|v1| !v1.pending(id))
            && self
                .v2
                .as_ref()
                .is_none_or(|v2| v2.external_barrier_ready(id))
    }
}
impl<'a> WorkloadSamplingBackend<dyn CgroupPersistence + 'a> for CgroupSamplingBackend {
    fn maintain(&mut self) {
        if let Some(v2) = self.v2.as_mut() {
            v2.maintain();
        }
    }
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut (dyn CgroupPersistence + 'a),
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let mut drain = ResourceMetricsDrain::default();
        if let Some(v2) = self.v2.as_mut() {
            drain = v2.collect(targets, storage, context, selection)?;
        }
        if let Some(v1) = self.v1.as_mut() {
            let next = v1.collect(targets, storage, context, selection)?;
            drain.samples.extend(next.samples);
            drain.failures.extend(next.failures);
        }
        Ok(drain)
    }
}

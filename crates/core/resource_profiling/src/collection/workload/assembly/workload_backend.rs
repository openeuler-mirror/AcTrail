use super::super::{
    cgroup::CgroupSamplingBackend, noop::NoOpSamplingBackend, procfs::ProcfsSamplingSessions,
    sampling::FallbackChain,
};
use crate::{
    CgroupPersistence, ProfilingError, ResourceMetricsDrain, SamplingContext, SamplingSelection,
    WorkloadSamplingBackend, WorkloadTargets,
};
pub struct WorkloadBackend {
    pub(super) selected: SelectedBackend,
}
pub(super) enum SelectedBackend {
    Procfs(ProcfsSamplingSessions),
    Cgroup(CgroupSamplingBackend),
    Auto(FallbackChain<CgroupSamplingBackend, ProcfsSamplingSessions>),
    NoOp(NoOpSamplingBackend),
}
impl WorkloadBackend {
    pub fn cgroup(&self) -> Option<&CgroupSamplingBackend> {
        match &self.selected {
            SelectedBackend::Cgroup(value) => Some(value),
            SelectedBackend::Auto(chain) => Some(&chain.primary),
            _ => None,
        }
    }
    pub fn cgroup_mut(&mut self) -> Option<&mut CgroupSamplingBackend> {
        match &mut self.selected {
            SelectedBackend::Cgroup(value) => Some(value),
            SelectedBackend::Auto(chain) => Some(&mut chain.primary),
            _ => None,
        }
    }
}
impl WorkloadSamplingBackend<dyn CgroupPersistence + '_> for WorkloadBackend {
    fn maintain(&mut self) {
        match &mut self.selected {
            SelectedBackend::Procfs(value) => <ProcfsSamplingSessions as WorkloadSamplingBackend<
                dyn CgroupPersistence,
            >>::maintain(value),
            SelectedBackend::Cgroup(value) => value.maintain(),
            SelectedBackend::Auto(value) => <FallbackChain<
                CgroupSamplingBackend,
                ProcfsSamplingSessions,
            > as WorkloadSamplingBackend<dyn CgroupPersistence>>::maintain(
                value
            ),
            SelectedBackend::NoOp(value) => <NoOpSamplingBackend as WorkloadSamplingBackend<
                dyn CgroupPersistence,
            >>::maintain(value),
        }
    }
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: &mut (dyn CgroupPersistence + '_),
        sampling: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        match &mut self.selected {
            SelectedBackend::Procfs(value) => value.collect(targets, context, sampling, selection),
            SelectedBackend::Cgroup(value) => value.collect(targets, context, sampling, selection),
            SelectedBackend::Auto(value) => value.collect(targets, context, sampling, selection),
            SelectedBackend::NoOp(value) => value.collect(targets, context, sampling, selection),
        }
    }
}

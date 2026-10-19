use crate::{
    ProfilingError, ResourceMetricsDrain, SamplingContext, SamplingSelection,
    WorkloadSamplingBackend, WorkloadTargets,
};
/// Explicitly selected empty backend: no I/O, samples, or lifecycle resources.
pub(crate) struct NoOpSamplingBackend;
impl<C: ?Sized> WorkloadSamplingBackend<C> for NoOpSamplingBackend {
    fn collect(
        &mut self,
        _targets: &dyn WorkloadTargets,
        _context: &mut C,
        _sampling: SamplingContext,
        _selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        Ok(ResourceMetricsDrain::default())
    }
}

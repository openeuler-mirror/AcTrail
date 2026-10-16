use super::{SamplingSelection, WorkloadSamplingBackend};
use crate::{ProfilingError, ResourceMetricsDrain, SamplingContext, WorkloadTargets};
pub(crate) struct FallbackChain<P, F> {
    pub(crate) primary: P,
    pub(crate) fallback: F,
}
impl<C: ?Sized, P: WorkloadSamplingBackend<C>, F: WorkloadSamplingBackend<C>>
    WorkloadSamplingBackend<C> for FallbackChain<P, F>
{
    fn maintain(&mut self) {
        self.primary.maintain();
        self.fallback.maintain();
    }
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: &mut C,
        sampling: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let mut drain = self
            .primary
            .collect(targets, context, sampling, selection)?;
        let fallback = self
            .fallback
            .collect(targets, context, sampling, selection)?;
        drain.samples.extend(fallback.samples);
        drain.failures.extend(fallback.failures);
        Ok(drain)
    }
}

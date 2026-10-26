use super::SamplingSelection;
use crate::{ProfilingError, ResourceMetricsDrain, SamplingContext, WorkloadTargets};
/// A backend owns its sessions and consumes a borrowed application context.
/// Skip targets already claimed this cycle. Claim owned targets before I/O,
/// including suspended or failed sessions, so a candidate cannot rebind them.
/// Report per-target failures in the drain; return Err only for batch failure.
pub trait WorkloadSamplingBackend<C: ?Sized> {
    fn maintain(&mut self) {}
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        context: &mut C,
        sampling: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError>;
}

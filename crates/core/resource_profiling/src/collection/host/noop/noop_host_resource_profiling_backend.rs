use crate::model::host_resource::HostResourceSample;
use crate::{HostResourceProfilingBackend, ResourceProfilingBackend, SamplingContext};
pub(crate) struct NoOpHostResourceProfilingBackend;
impl ResourceProfilingBackend for NoOpHostResourceProfilingBackend {
    type Input = ();
    type Sample = HostResourceSample;
    fn sample(&mut self, _: &(), _: SamplingContext) -> Result<Option<HostResourceSample>, String> {
        Ok(None)
    }
}
impl HostResourceProfilingBackend for NoOpHostResourceProfilingBackend {}

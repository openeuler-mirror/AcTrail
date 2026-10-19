use crate::model::host_resource::HostResourceSample;

use crate::ResourceProfilingBackend;

pub trait HostResourceProfilingBackend:
    ResourceProfilingBackend<Input = (), Sample = HostResourceSample>
{
}

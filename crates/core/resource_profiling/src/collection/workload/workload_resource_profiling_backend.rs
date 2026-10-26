use crate::ResourceProfilingBackend;
use model_core::process::ProcessIdentity;

pub trait WorkloadResourceProfilingBackend: ResourceProfilingBackend {
    fn root_identity(&self) -> ProcessIdentity;
}

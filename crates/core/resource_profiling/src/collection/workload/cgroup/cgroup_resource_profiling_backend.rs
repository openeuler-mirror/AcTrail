use crate::WorkloadResourceProfilingBackend;
/// A bound cgroup version owns its counters and directory identities.
pub trait CgroupResourceProfilingBackend: WorkloadResourceProfilingBackend {}

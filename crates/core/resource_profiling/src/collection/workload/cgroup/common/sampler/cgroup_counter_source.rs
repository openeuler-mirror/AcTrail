use linux_cgroup::{CgroupCounters, CgroupError};

/// Counter access to one cgroup; implemented per version next to its adapter.
pub trait CgroupCounterSource {
    fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError>;
    fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError>;
}

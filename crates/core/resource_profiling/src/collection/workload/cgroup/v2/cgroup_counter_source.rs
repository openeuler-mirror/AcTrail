use super::external::ExternalScopeBinding;
use linux_cgroup::{CgroupCounters, CgroupError, CgroupV2CounterReader};

pub trait CgroupCounterSource {
    fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError>;
    fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError>;
}

impl CgroupCounterSource for CgroupV2CounterReader {
    fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError> {
        self.read_counters()
    }
    fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError> {
        self.read_subtree_pids()
    }
}

impl CgroupCounterSource for ExternalScopeBinding {
    fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError> {
        self.read_counters()
    }
    fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError> {
        self.read_subtree_pids()
    }
}

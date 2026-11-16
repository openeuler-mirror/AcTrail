mod config;
pub use config::{CgroupV2BindingConfig, CgroupV2BindingPolicy};
mod handle;
pub use handle::{CgroupV2BindingHandle, ContainerCgroupResolver};
mod manager;
mod persistence;
pub(crate) use manager::{
    CgroupV2BindingAdmission, CgroupV2BindingFinalRead, CgroupV2BindingManager, CgroupV2BindingRead,
};
pub use persistence::{CgroupV2BindingPersistence, CgroupV2BindingRecovery};

impl crate::collection::workload::cgroup::common::sampler::CgroupCounterSource
    for CgroupV2BindingHandle
{
    fn read_counters(&mut self) -> Result<linux_cgroup::CgroupCounters, linux_cgroup::CgroupError> {
        self.read_counters()
    }
    fn read_subtree_pids(&self) -> Result<Vec<u32>, linux_cgroup::CgroupError> {
        self.read_subtree_pids()
    }
}

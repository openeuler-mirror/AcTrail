//! TLS probe preparation for a stopped launch process.

use control_contract::command::LaunchTlsProbePlan;
use control_contract::reply::ControlError;
use ebpf_collector::loader::DynamicTlsProbePlan;

use super::{PinnedRuntimePath, RuntimeRootPathMapper, TlsSyncService};

pub(in crate::services) struct PinnedLaunchPlan {
    descriptor: LaunchTlsProbePlan,
    target: PinnedRuntimePath,
    binary: PinnedRuntimePath,
}

impl PinnedLaunchPlan {
    fn new(
        mapper: &RuntimeRootPathMapper,
        descriptor: LaunchTlsProbePlan,
    ) -> Result<Self, ControlError> {
        Ok(Self {
            target: mapper.pin(&descriptor.target)?,
            binary: mapper.pin(&descriptor.binary)?,
            descriptor,
        })
    }

    pub(in crate::services) fn dynamic_plan(&self) -> DynamicTlsProbePlan {
        DynamicTlsProbePlan {
            target: self.target.path().to_path_buf(),
            target_identity: self.descriptor.target_identity.clone(),
            binary: self.binary.path().to_path_buf(),
            binary_identity: self.descriptor.binary_identity.clone(),
            provider: self.descriptor.provider.clone(),
            points: self.descriptor.points.clone(),
        }
    }
}

impl TlsSyncService {
    pub(in crate::services) fn startup_discovery_enabled(&self) -> bool {
        self.resolver
            .as_ref()
            .is_some_and(|resolver| resolver.startup_discovery_enabled())
    }

    pub(in crate::services) fn pin_launch_plans(
        &self,
        host_pid: u32,
        plans: Vec<LaunchTlsProbePlan>,
    ) -> Result<Vec<PinnedLaunchPlan>, ControlError> {
        if plans.is_empty() {
            return Ok(Vec::new());
        }
        let mapper = self.runtime_root_path_mapper(host_pid)?;
        plans
            .into_iter()
            .map(|plan| PinnedLaunchPlan::new(&mapper, plan))
            .collect()
    }
}

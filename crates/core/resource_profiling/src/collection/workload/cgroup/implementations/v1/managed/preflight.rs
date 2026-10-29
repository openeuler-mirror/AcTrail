use super::ManagedV1ScopeConfig;
use linux_cgroup::{CgroupError, ManagedCgroupV1, V1PreflightReport};

pub(crate) struct ManagedV1Preflight;
impl ManagedV1Preflight {
    /// Discovers the v1 hierarchies and creates `<mount>/<root_name>/traces` in each.
    pub(crate) fn prepare(
        config: &ManagedV1ScopeConfig,
    ) -> Result<(ManagedCgroupV1, V1PreflightReport), CgroupError> {
        let adapter = ManagedCgroupV1::discover(&config.root_name)?;
        let report = adapter.prepare_hierarchy()?;
        Ok((adapter, report))
    }
}

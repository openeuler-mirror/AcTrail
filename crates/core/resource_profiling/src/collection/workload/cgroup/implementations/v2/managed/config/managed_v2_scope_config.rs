use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedV2ScopeConfig {
    pub root: std::path::PathBuf,
    pub recovery_scan_limit: u32,
    pub finalization_timeout_ms: u64,
}
impl Default for ManagedV2ScopeConfig {
    fn default() -> Self {
        Self {
            root: "/sys/fs/cgroup/actrail".into(),
            recovery_scan_limit: 1024,
            finalization_timeout_ms: 30_000,
        }
    }
}
impl ManagedV2ScopeConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let config: Self = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| {
                format!("resource_profiling.collection.workload.cgroup.v2.managed: {error}")
            })?;
        if !config.root.is_absolute() || config.root == std::path::Path::new("/") {
            return Err(
                "resource_profiling.collection.workload.cgroup.v2.managed.root must be an absolute non-root path"
                    .into(),
            );
        }
        if config.recovery_scan_limit == 0 {
            return Err(
                "resource_profiling.collection.workload.cgroup.v2.managed.recovery_scan_limit must be positive"
                    .into(),
            );
        }
        if config.finalization_timeout_ms == 0
            || std::time::Instant::now()
                .checked_add(std::time::Duration::from_millis(
                    config.finalization_timeout_ms,
                ))
                .is_none()
        {
            return Err("resource_profiling.collection.workload.cgroup.v2.managed.finalization_timeout_ms must be positive and representable".into());
        }
        Ok(config)
    }
}

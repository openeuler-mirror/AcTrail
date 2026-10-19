use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CgroupV1Config {
    pub binding_attempts: u32,
    pub allow_hierarchy_root: bool,
}
impl Default for CgroupV1Config {
    fn default() -> Self {
        Self {
            binding_attempts: 2,
            allow_hierarchy_root: false,
        }
    }
}
impl CgroupV1Config {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let config: Self = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource_profiling.collection.workload.cgroup.v1: {e}"))?;
        if config.binding_attempts == 0 {
            return Err(
                "resource_profiling.collection.workload.cgroup.v1.binding_attempts must be positive".into(),
            );
        }
        Ok(config)
    }
}

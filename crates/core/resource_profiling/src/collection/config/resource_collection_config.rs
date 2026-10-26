use crate::{HostProfilingConfig, WorkloadProfilingConfig};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ResourceCollectionConfig {
    pub host: HostProfilingConfig,
    pub workload: WorkloadProfilingConfig,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    host: toml::Table,
    workload: toml::Table,
}
impl ResourceCollectionConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource_profiling.collection: {e}"))?;
        Ok(Self {
            host: HostProfilingConfig::parse(&raw.host)?,
            workload: WorkloadProfilingConfig::parse(&raw.workload)?,
        })
    }
}

use super::super::{
    external::ExternalScopeConfig, managed::ManagedScopeConfig, process_rss::CgroupProcessRssConfig,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CgroupV2Config {
    pub managed: ManagedScopeConfig,
    pub external: ExternalScopeConfig,
    pub process_rss: CgroupProcessRssConfig,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    managed: toml::Table,
    external: toml::Table,
    process_rss: toml::Table,
}
impl CgroupV2Config {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| {
                format!("resource_profiling.collection.workload.cgroup.v2: {error}")
            })?;
        Ok(Self {
            managed: ManagedScopeConfig::parse(&raw.managed)?,
            external: ExternalScopeConfig::parse(&raw.external)?,
            process_rss: CgroupProcessRssConfig::parse(&raw.process_rss)?,
        })
    }
}

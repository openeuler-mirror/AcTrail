use super::super::{binding::CgroupV2BindingConfig, managed::ManagedV2ScopeConfig};
use crate::collection::workload::cgroup::common::sampler::CgroupProcessRssConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CgroupV2Config {
    pub managed: ManagedV2ScopeConfig,
    /// Container cgroup binding settings. The TOML key stays `external`.
    pub external: CgroupV2BindingConfig,
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
            managed: ManagedV2ScopeConfig::parse(&raw.managed)?,
            external: CgroupV2BindingConfig::parse(&raw.external)?,
            process_rss: CgroupProcessRssConfig::parse(&raw.process_rss)?,
        })
    }
}

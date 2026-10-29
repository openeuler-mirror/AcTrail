use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CgroupProcessRssConfig {
    pub enabled: bool,
}
impl Default for CgroupProcessRssConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}
impl CgroupProcessRssConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let config: Self = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| {
                format!("resource_profiling.collection.workload.cgroup.v2.process_rss: {error}")
            })?;

        Ok(config)
    }
}

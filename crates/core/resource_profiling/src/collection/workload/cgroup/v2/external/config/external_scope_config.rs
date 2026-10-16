use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalScopePolicy {
    #[default]
    Disabled,
    Prefer,
    Require,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExternalScopeConfig {
    pub policy: ExternalScopePolicy,
    pub failure_threshold: u32,
    pub binding_attempts: u32,
}
impl Default for ExternalScopeConfig {
    fn default() -> Self {
        Self {
            policy: ExternalScopePolicy::Disabled,
            failure_threshold: 3,
            binding_attempts: 2,
        }
    }
}
impl ExternalScopeConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let config: Self = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| {
                format!("resource_profiling.collection.workload.cgroup.v2.external: {error}")
            })?;
        if config.policy != ExternalScopePolicy::Disabled && config.failure_threshold == 0 {
            return Err(
                "resource_profiling.collection.workload.cgroup.v2.external.failure_threshold must be positive"
                    .into(),
            );
        }
        if config.policy != ExternalScopePolicy::Disabled && config.binding_attempts == 0 {
            return Err(
                "resource_profiling.collection.workload.cgroup.v2.external.binding_attempts must be positive"
                    .into(),
            );
        }
        Ok(config)
    }
}

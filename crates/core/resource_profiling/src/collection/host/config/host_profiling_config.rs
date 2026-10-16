use super::HostBackendMode;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostProfilingConfig {
    pub enabled: bool,
    pub interval_ms: u64,
    pub mode: HostBackendMode,
}
impl Default for HostProfilingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_ms: 1000,
            mode: HostBackendMode::Linux,
        }
    }
}
impl HostProfilingConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| format!("host/config: {error}"))?;
        if !raw.enabled {
            return Ok(Self {
                enabled: false,
                ..Self::default()
            });
        }
        let config = Self {
            enabled: true,
            mode: raw
                .mode
                .try_into()
                .map_err(|error| format!("resource_profiling.collection.host.mode: {error}"))?,
            interval_ms: raw.interval_ms.try_into().map_err(|error| {
                format!("resource_profiling.collection.host.interval_ms: {error}")
            })?,
        };
        if config.enabled && config.interval_ms == 0 {
            return Err("resource_profiling.collection.host.interval_ms must be positive".into());
        }
        if config.enabled
            && std::time::Instant::now()
                .checked_add(std::time::Duration::from_millis(config.interval_ms))
                .is_none()
        {
            return Err(
                "resource_profiling.collection.host.interval_ms exceeds the monotonic clock range"
                    .into(),
            );
        }
        Ok(config)
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    enabled: bool,
    interval_ms: toml::Value,
    mode: toml::Value,
}

impl Default for RawConfig {
    fn default() -> Self {
        let config = HostProfilingConfig::default();
        Self {
            enabled: config.enabled,
            mode: toml::Value::String("linux".into()),
            interval_ms: toml::Value::Integer(config.interval_ms as i64),
        }
    }
}

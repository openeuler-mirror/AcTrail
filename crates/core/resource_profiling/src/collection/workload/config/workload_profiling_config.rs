use super::super::{cgroup::CgroupProfilingConfig, procfs::ProcfsWorkloadProfilingConfig};
use super::WorkloadBackendMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkloadProfilingConfig {
    pub enabled: bool,
    pub interval_ms: u64,
    pub mode: WorkloadBackendMode,
    pub procfs: Option<ProcfsWorkloadProfilingConfig>,
    pub cgroup: Option<CgroupProfilingConfig>,
}
impl Default for WorkloadProfilingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_ms: 1000,
            mode: WorkloadBackendMode::Auto,
            procfs: Some(ProcfsWorkloadProfilingConfig::default()),
            cgroup: Some(CgroupProfilingConfig::default()),
        }
    }
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    enabled: bool,
    interval_ms: toml::Value,
    mode: toml::Value,
    procfs: toml::Table,
    cgroup: toml::Table,
}
impl Default for RawConfig {
    fn default() -> Self {
        let config = WorkloadProfilingConfig::default();
        Self {
            enabled: config.enabled,
            interval_ms: toml::Value::Integer(config.interval_ms as i64),
            mode: toml::Value::String("auto".into()),
            procfs: toml::Table::new(),
            cgroup: toml::Table::new(),
        }
    }
}
impl WorkloadProfilingConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| format!("resource_profiling.collection.workload: {error}"))?;
        if !raw.enabled {
            return Ok(Self {
                enabled: false,
                procfs: None,
                cgroup: None,
                ..Self::default()
            });
        }
        let interval_ms: u64 = raw.interval_ms.try_into().map_err(|error| {
            format!("resource_profiling.collection.workload.interval_ms: {error}")
        })?;
        let mode: WorkloadBackendMode = raw
            .mode
            .try_into()
            .map_err(|error| format!("resource_profiling.collection.workload.mode: {error}"))?;
        if interval_ms == 0 {
            return Err(
                "resource_profiling.collection.workload.interval_ms must be positive".into(),
            );
        }
        Ok(Self {
            enabled: raw.enabled,
            interval_ms,
            mode,
            procfs: if matches!(
                mode,
                WorkloadBackendMode::Auto | WorkloadBackendMode::Procfs
            ) {
                Some(ProcfsWorkloadProfilingConfig::parse(&raw.procfs)?)
            } else {
                None
            },
            cgroup: if matches!(
                mode,
                WorkloadBackendMode::Auto | WorkloadBackendMode::Cgroup
            ) {
                Some(CgroupProfilingConfig::parse(&raw.cgroup)?)
            } else {
                None
            },
        })
    }
}

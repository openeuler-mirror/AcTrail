use super::super::{v1::CgroupV1Config, v2::CgroupV2Config};
use super::CgroupBackendMode;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CgroupProfilingConfig {
    pub mode: CgroupBackendMode,
    pub v1: Option<CgroupV1Config>,
    pub v2: Option<CgroupV2Config>,
}
impl Default for CgroupProfilingConfig {
    fn default() -> Self {
        Self {
            mode: CgroupBackendMode::Auto,
            v1: Some(CgroupV1Config::default()),
            v2: Some(CgroupV2Config::default()),
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    mode: CgroupBackendMode,
    v1: toml::Table,
    v2: toml::Table,
}
impl CgroupProfilingConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource_profiling.collection.workload.cgroup: {e}"))?;
        Ok(Self {
            mode: raw.mode,
            v1: if raw.mode != CgroupBackendMode::V2 {
                Some(CgroupV1Config::parse(&raw.v1)?)
            } else {
                None
            },
            v2: if raw.mode != CgroupBackendMode::V1 {
                Some(CgroupV2Config::parse(&raw.v2)?)
            } else {
                None
            },
        })
    }
}

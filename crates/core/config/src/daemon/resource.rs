//! Resource metric sampler configuration.

use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResourceMetricsMode {
    #[default]
    Procfs,
    CgroupV2,
    Auto,
}

impl ResourceMetricsMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Procfs => "procfs",
            Self::CgroupV2 => "cgroup-v2",
            Self::Auto => "auto",
        }
    }
}

impl FromStr for ResourceMetricsMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "procfs" => Ok(Self::Procfs),
            "cgroup-v2" => Ok(Self::CgroupV2),
            "auto" => Ok(Self::Auto),
            other => Err(format!(
                "invalid resource metrics mode {other}; expected procfs, cgroup-v2, or auto"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExistingContainerCgroups {
    #[default]
    Disabled,
    Prefer,
    Require,
}

impl ExistingContainerCgroups {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Prefer => "prefer",
            Self::Require => "require",
        }
    }
}

impl FromStr for ExistingContainerCgroups {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "prefer" => Ok(Self::Prefer),
            "require" => Ok(Self::Require),
            other => Err(format!(
                "invalid existing container cgroup policy {other}; expected disabled, prefer, or require"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceMetricsConfig {
    pub enabled: bool,
    pub mode: ResourceMetricsMode,
    pub existing_container_cgroups: ExistingContainerCgroups,
    pub external_cgroup_failure_threshold: u32,
    pub interval_ms: u64,
    pub include_children: bool,
    pub include_system: bool,
    pub cgroup_root: PathBuf,
    pub finalization_timeout_ms: u64,
    pub orphan_limit: u32,
    pub cpu_alert_percent_millis: Option<u64>,
    pub memory_alert_rss_kb: Option<u64>,
    pub memory_alert_current_bytes: Option<u64>,
}

impl Default for ResourceMetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: ResourceMetricsMode::Procfs,
            existing_container_cgroups: ExistingContainerCgroups::Disabled,
            external_cgroup_failure_threshold: 3,
            interval_ms: 1_000,
            include_children: true,
            include_system: true,
            cgroup_root: PathBuf::from("/sys/fs/cgroup/actrail"),
            finalization_timeout_ms: 30_000,
            orphan_limit: 1_024,
            cpu_alert_percent_millis: None,
            memory_alert_rss_kb: None,
            memory_alert_current_bytes: None,
        }
    }
}

impl ResourceMetricsConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceAccountingMethod {
    CgroupV1,
    CgroupV2,
    #[default]
    ProcfsRssSum,
}

impl ResourceAccountingMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CgroupV1 => "cgroup_v1",
            Self::CgroupV2 => "cgroup_v2",
            Self::ProcfsRssSum => "procfs_rss_sum",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceAccountingCoverage {
    Exact,
    BroaderThanTrace,
    #[default]
    Partial,
}

impl ResourceAccountingCoverage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::BroaderThanTrace => "broader_than_trace",
            Self::Partial => "partial",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSampleKind {
    #[default]
    Periodic,
    Final,
}

impl ResourceSampleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::Final => "final",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryEventCounters {
    pub low: Option<u64>,
    pub high: Option<u64>,
    pub max: Option<u64>,
    pub oom: Option<u64>,
    pub oom_kill: Option<u64>,
    pub oom_group_kill: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub scope: String,
    pub subject: String,
    #[serde(default)]
    pub accounting_method: ResourceAccountingMethod,
    #[serde(default)]
    pub accounting_coverage: ResourceAccountingCoverage,
    #[serde(default)]
    pub sample_kind: ResourceSampleKind,
    pub cpu_percent_millis: Option<u64>,
    pub rss_kb: Option<u64>,
    pub virtual_memory_kb: Option<u64>,
    pub memory_current_bytes: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    pub memory_anon_bytes: Option<u64>,
    pub memory_file_bytes: Option<u64>,
    pub memory_swap_current_bytes: Option<u64>,
    pub memory_events: Option<MemoryEventCounters>,
    pub memory_events_local: Option<MemoryEventCounters>,
    pub cpu_usage_usec: Option<u64>,
    pub cpu_user_usec: Option<u64>,
    pub cpu_system_usec: Option<u64>,
    pub cpu_nr_throttled: Option<u64>,
    pub cpu_throttled_usec: Option<u64>,
    pub io_read_bytes: Option<u64>,
    pub io_write_bytes: Option<u64>,
    pub pids_current: Option<u64>,
    pub pids_peak: Option<u64>,
    pub process_rss_sum_kb: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkloadResourceSample {
    #[serde(with = "super::serde_identity::TraceIdentity")]
    pub trace_id: model_core::ids::TraceId,
    pub observed_at: std::time::SystemTime,
    pub process: model_core::process::ProcessIdentity,
    pub payload: ResourceMetrics,
}

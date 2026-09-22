//! Event envelope and payload families used after ingest normalization.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::ids::{CollectorName, EventId, TraceId};
use crate::policy::{PolicyRecord, PolicyVerdict};
use crate::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Process,
    File,
    Net,
    Ipc,
    Stdio,
    Application,
    Resource,
    Control,
    Loss,
    Label,
    Enforcement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventFlags {
    pub bootstrap_observed: bool,
    pub metadata_partial: bool,
    pub policy_modified: bool,
}

impl EventFlags {
    pub const fn clean() -> Self {
        Self {
            bootstrap_observed: false,
            metadata_partial: false,
            policy_modified: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub collector: CollectorName,
    pub kind: EventKind,
    pub flags: EventFlags,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProcessPayload {
    pub operation: String,
    pub parent: Option<ProcessIdentity>,
    pub executable: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FilePayload {
    pub operation: String,
    pub path: Option<String>,
    pub result: Option<i32>,
    pub metadata: BTreeMap<String, String>,
    pub io_summary: Option<FileIoSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIoDirection {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIoTargetKind {
    RegularFile,
    CharacterDevice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileSummaryPathState {
    Resolved,
    Truncated,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileIoSummary {
    pub file_token: u64,
    pub direction: FileIoDirection,
    pub target_kind: FileIoTargetKind,
    pub errno: u32,
    pub interval_start: SystemTime,
    pub interval_end: SystemTime,
    pub interval_complete: bool,
    pub operations: Option<u64>,
    pub bytes: Option<u64>,
    pub path_state: FileSummaryPathState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetPayload {
    pub transport: String,
    pub local: Option<String>,
    pub remote: Option<String>,
    pub size: Option<u64>,
    pub result: Option<i32>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IpcPayload {
    pub channel: String,
    pub peer: Option<String>,
    pub size: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StdioPayload {
    pub stream: String,
    pub data: Vec<u8>,
    pub original_size: Option<usize>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ApplicationBody {
    Text(String),
    Json(String),
    Base64(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplicationPayload {
    pub protocol: String,
    pub operation: String,
    pub summary: String,
    pub body: Option<ApplicationBody>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceAccountingMethod {
    CgroupV2,
    #[default]
    ProcfsRssSum,
}

impl ResourceAccountingMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CgroupV2 => "cgroup_v2",
            Self::ProcfsRssSum => "procfs_rss_sum",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceAccountingCoverage {
    Exact,
    /// Used by the subsequent existing-container PR, not managed host scopes.
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
pub struct ResourcePayload {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlPayload {
    pub action: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LossPayload {
    pub reason: String,
    pub fatal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LabelPayload {
    pub provider: String,
    pub confidence_millis: Option<u16>,
    pub evidence: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnforcementPayload {
    pub backend: String,
    pub operation: String,
    pub decision: String,
    pub path: Option<String>,
    pub rule_id: Option<String>,
    pub result: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventPayload {
    Process(ProcessPayload),
    File(FilePayload),
    Net(NetPayload),
    Ipc(IpcPayload),
    Stdio(StdioPayload),
    Application(ApplicationPayload),
    Resource(ResourcePayload),
    Control(ControlPayload),
    Loss(LossPayload),
    Label(LabelPayload),
    Enforcement(EnforcementPayload),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainEvent {
    pub envelope: EventEnvelope,
    pub payload: EventPayload,
    pub policy: PolicyRecord,
}

impl DomainEvent {
    pub fn new(envelope: EventEnvelope, payload: EventPayload) -> Self {
        Self {
            envelope,
            payload,
            policy: PolicyRecord::allow(),
        }
    }

    pub fn with_policy(mut self, policy: PolicyRecord) -> Self {
        self.envelope.flags.policy_modified = policy.verdict != PolicyVerdict::Allow;
        self.policy = policy;
        self
    }
}

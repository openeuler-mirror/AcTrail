//! Event envelope and payload families used after ingest normalization.

use std::collections::BTreeMap;
use std::time::SystemTime;

use crate::ids::{CollectorName, EventId, TraceId};
use crate::policy::{PolicyRecord, PolicyVerdict};
use crate::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessPayload {
    pub operation: String,
    pub parent: Option<ProcessIdentity>,
    pub executable: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePayload {
    pub operation: String,
    pub path: Option<String>,
    pub result: Option<i32>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetPayload {
    pub transport: String,
    pub local: Option<String>,
    pub remote: Option<String>,
    pub size: Option<u64>,
    pub result: Option<i32>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpcPayload {
    pub channel: String,
    pub peer: Option<String>,
    pub size: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StdioPayload {
    pub stream: String,
    pub data: Vec<u8>,
    pub original_size: Option<usize>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPayload {
    pub protocol: String,
    pub operation: String,
    pub summary: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourcePayload {
    pub scope: String,
    pub subject: String,
    pub cpu_percent_millis: Option<u64>,
    pub rss_kb: Option<u64>,
    pub virtual_memory_kb: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlPayload {
    pub action: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LossPayload {
    pub reason: String,
    pub fatal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabelPayload {
    pub provider: String,
    pub confidence_millis: Option<u16>,
    pub evidence: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnforcementPayload {
    pub backend: String,
    pub operation: String,
    pub decision: String,
    pub path: Option<String>,
    pub rule_id: Option<String>,
    pub result: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

use crate::{PluginPurpose, PluginRuntimeKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginLifecycleState {
    Active,
    Draining,
    Stopped,
    Failed,
}

impl PluginLifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "active" => Ok(Self::Active),
            "draining" => Ok(Self::Draining),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("unknown plugin lifecycle state {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PluginHostcallMetrics {
    pub payload_read: PluginPayloadReadMetrics,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PluginPayloadReadMetrics {
    pub calls: u64,
    pub bytes: u64,
    pub denied: u64,
    pub not_found: u64,
    pub invalid: u64,
    pub too_large: u64,
    pub truncated: u64,
    pub latency_total_ns: u64,
    pub latency_max_ns: u64,
}

pub trait PluginHostcallMetricsSource: Send + Sync {
    fn snapshot(&self) -> PluginHostcallMetrics;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginInstanceStatus {
    pub instance_id: String,
    pub plugin_id: String,
    pub purpose: PluginPurpose,
    pub runtime: PluginRuntimeKind,
    pub state: PluginLifecycleState,
    pub host_grants: Vec<String>,
    pub queue_depth: Option<u64>,
    pub queue_capacity: Option<u32>,
    pub observed_records: u64,
    pub dropped_records: u64,
    pub hostcall_metrics: PluginHostcallMetrics,
    pub last_error: Option<String>,
    pub warnings: Vec<String>,
}

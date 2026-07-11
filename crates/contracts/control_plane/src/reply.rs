//! Control-plane reply and error contracts.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::command::DeploymentPermissionMode;
use model_core::capability::Capability;
use model_core::ids::{ProfileName, TraceId, TraceName};
use model_core::trace::{TraceHealth, TraceLifecycleState};
use plugin_system::PluginInstanceStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceListItem {
    pub trace_id: TraceId,
    pub display_name: TraceName,
    pub root_pid: u32,
    pub lifecycle_state: TraceLifecycleState,
    pub health: TraceHealth,
    pub tags: BTreeSet<String>,
    pub created_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackAddReply {
    pub trace_id: TraceId,
    pub lifecycle_state: TraceLifecycleState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchPermissionsReply {
    pub requested_host_ebpf: DeploymentPermissionMode,
    pub requested_seccomp_notify: DeploymentPermissionMode,
    pub selected_host_ebpf: bool,
    pub selected_seccomp_notify: bool,
    pub selected_profile_name: ProfileName,
    pub payload_tls_seccomp: bool,
    pub payload_socket_seccomp: bool,
    pub process_seccomp: bool,
    pub network_control_seccomp: bool,
    pub required_capabilities: Vec<Capability>,
    pub degraded: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchTlsPlanDescriptor {
    pub target: PathBuf,
    pub binary: PathBuf,
    pub provider: String,
    pub source: String,
    pub points: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchTlsPlanStatus {
    Found(LaunchTlsPlanDescriptor),
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchTlsPlanReply {
    pub status: LaunchTlsPlanStatus,
    pub cache_hit: bool,
    pub resolve_elapsed_micros: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorReply {
    pub available_collectors: Vec<String>,
    pub loaded_policy_plugins: Vec<String>,
    pub storage_ready: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCommandReply {
    pub instance_id: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlReply {
    LaunchPermissions(LaunchPermissionsReply),
    LaunchTlsPlan(LaunchTlsPlanReply),
    TrackAdded(TrackAddReply),
    SeccompListenerRegistered,
    TrackRemoved,
    TraceList(Vec<TraceListItem>),
    Doctor(DoctorReply),
    PluginList(Vec<PluginInstanceStatus>),
    PluginStatus(PluginInstanceStatus),
    PluginCommand(PluginCommandReply),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlError {
    pub code: String,
    pub message: String,
}

impl ControlError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

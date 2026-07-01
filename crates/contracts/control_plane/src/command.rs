//! Control-plane command contracts.

use std::collections::BTreeSet;
use std::os::fd::RawFd;

use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
use model_core::process::{InitialSuppressedFd, NamespaceIdentity};

use crate::selector::TraceSelector;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRef {
    pub namespace_pid: u32,
    pub pid_namespace: NamespaceIdentity,
}

impl ProcessRef {
    pub fn new(namespace_pid: u32, pid_namespace: NamespaceIdentity) -> Self {
        Self {
            namespace_pid,
            pid_namespace,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackAddCommand {
    pub request_id: RequestId,
    pub root: ProcessRef,
    pub display_name: TraceName,
    pub profile_name: ProfileName,
    pub tags: BTreeSet<String>,
    pub launch_mode: bool,
    pub initial_suppressed_fds: Vec<InitialSuppressedFd>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterSeccompListenerCommand {
    pub request_id: RequestId,
    pub trace_id: TraceId,
    pub target: ProcessRef,
    pub listener_fd: Option<RawFd>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackRemoveCommand {
    pub request_id: RequestId,
    pub selector: TraceSelector,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListTracesCommand {
    pub request_id: RequestId,
    pub selector: Option<TraceSelector>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorCommand {
    pub request_id: RequestId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginListCommand {
    pub request_id: RequestId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginStatusCommand {
    pub request_id: RequestId,
    pub instance_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginLoadCommand {
    pub request_id: RequestId,
    pub manifest_path: String,
    pub plugin_config_path: Option<String>,
    pub instance_id: String,
    pub host_grants: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginUnloadCommand {
    pub request_id: RequestId,
    pub instance_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCommandCommand {
    pub request_id: RequestId,
    pub instance_id: String,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlCommand {
    TrackAdd(TrackAddCommand),
    RegisterSeccompListener(RegisterSeccompListenerCommand),
    TrackRemove(TrackRemoveCommand),
    ListTraces(ListTracesCommand),
    Doctor(DoctorCommand),
    PluginList(PluginListCommand),
    PluginStatus(PluginStatusCommand),
    PluginLoad(PluginLoadCommand),
    PluginUnload(PluginUnloadCommand),
    PluginCommand(PluginCommandCommand),
}

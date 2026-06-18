//! Control-plane command contracts.

use std::collections::BTreeSet;
use std::os::fd::RawFd;

use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
use model_core::process::InitialSuppressedFd;

use crate::selector::TraceSelector;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackAddCommand {
    pub request_id: RequestId,
    pub root_pid: u32,
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
    pub target_pid: u32,
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
pub enum ControlCommand {
    TrackAdd(TrackAddCommand),
    RegisterSeccompListener(RegisterSeccompListenerCommand),
    TrackRemove(TrackRemoveCommand),
    ListTraces(ListTracesCommand),
    Doctor(DoctorCommand),
}

//! Control-plane reply and error contracts.

use std::collections::BTreeSet;
use std::time::SystemTime;

use model_core::ids::{TraceId, TraceName};
use model_core::trace::{TraceHealth, TraceLifecycleState};

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
pub struct DoctorReply {
    pub available_collectors: Vec<String>,
    pub loaded_policy_plugins: Vec<String>,
    pub storage_ready: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlReply {
    TrackAdded(TrackAddReply),
    SeccompListenerRegistered,
    TrackRemoved,
    TraceList(Vec<TraceListItem>),
    Doctor(DoctorReply),
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

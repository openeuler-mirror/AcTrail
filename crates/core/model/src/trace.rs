//! Trace lifecycle, health, and snapshot-ready runtime records.

use std::collections::BTreeSet;
use std::time::SystemTime;

use crate::ids::{ProfileName, TraceId, TraceName};
use crate::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceLifecycleState {
    Starting,
    Active,
    Draining,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceHealth {
    Clean,
    Degraded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceTiming {
    pub created_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub completed_at: Option<SystemTime>,
    pub failed_at: Option<SystemTime>,
}

impl TraceTiming {
    pub fn new(created_at: SystemTime) -> Self {
        Self {
            created_at,
            started_at: None,
            completed_at: None,
            failed_at: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRecord {
    pub trace_id: TraceId,
    pub root_process_identity: ProcessIdentity,
    pub display_name: TraceName,
    pub profile_name: ProfileName,
    pub tags: BTreeSet<String>,
    pub lifecycle_state: TraceLifecycleState,
    pub health: TraceHealth,
    pub timings: TraceTiming,
}

impl TraceRecord {
    pub fn new(
        trace_id: TraceId,
        root_process_identity: ProcessIdentity,
        display_name: TraceName,
        profile_name: ProfileName,
        created_at: SystemTime,
    ) -> Self {
        Self {
            trace_id,
            root_process_identity,
            display_name,
            profile_name,
            tags: BTreeSet::new(),
            lifecycle_state: TraceLifecycleState::Starting,
            health: TraceHealth::Clean,
            timings: TraceTiming::new(created_at),
        }
    }

    pub fn add_tag(&mut self, tag: impl Into<String>) {
        self.tags.insert(tag.into());
    }
}

//! Trace lifecycle, health, and snapshot-ready runtime records.

use std::collections::BTreeSet;
use std::fmt;
use std::time::SystemTime;

use crate::ids::{ProfileName, TraceId, TraceName};
use crate::process::ProcessIdentity;

const TRACE_ALERT_TOKEN_BYTES: usize = 32;

#[derive(Clone, Eq, PartialEq)]
pub struct TraceAlertToken([u8; TRACE_ALERT_TOKEN_BYTES]);

impl TraceAlertToken {
    pub const BYTE_COUNT: usize = TRACE_ALERT_TOKEN_BYTES;

    pub const fn new(bytes: [u8; TRACE_ALERT_TOKEN_BYTES]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; TRACE_ALERT_TOKEN_BYTES] {
        &self.0
    }

    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }
}

impl fmt::Debug for TraceAlertToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TraceAlertToken([redacted])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceLifecycleState {
    Starting,
    Active,
    Draining,
    Completed,
    Exited,
    Failed,
}

impl TraceLifecycleState {
    pub const STORAGE_STARTING: &'static str = "starting";
    pub const STORAGE_ACTIVE: &'static str = "active";
    pub const STORAGE_DRAINING: &'static str = "draining";
    pub const STORAGE_COMPLETED: &'static str = "completed";
    pub const STORAGE_EXITED: &'static str = "exited";
    pub const STORAGE_FAILED: &'static str = "failed";

    pub const DISPLAY_STARTING: &'static str = "Starting";
    pub const DISPLAY_ACTIVE: &'static str = "Active";
    pub const DISPLAY_DRAINING: &'static str = "Draining";
    pub const DISPLAY_COMPLETED: &'static str = "Completed";
    pub const DISPLAY_EXITED: &'static str = "Exited";
    pub const DISPLAY_FAILED: &'static str = "Failed";

    pub const fn as_storage_str(self) -> &'static str {
        match self {
            Self::Starting => Self::STORAGE_STARTING,
            Self::Active => Self::STORAGE_ACTIVE,
            Self::Draining => Self::STORAGE_DRAINING,
            Self::Completed => Self::STORAGE_COMPLETED,
            Self::Exited => Self::STORAGE_EXITED,
            Self::Failed => Self::STORAGE_FAILED,
        }
    }

    pub fn from_storage_str(raw: &str) -> Option<Self> {
        match raw {
            Self::STORAGE_STARTING => Some(Self::Starting),
            Self::STORAGE_ACTIVE => Some(Self::Active),
            Self::STORAGE_DRAINING => Some(Self::Draining),
            Self::STORAGE_COMPLETED => Some(Self::Completed),
            Self::STORAGE_EXITED => Some(Self::Exited),
            Self::STORAGE_FAILED => Some(Self::Failed),
            _ => None,
        }
    }

    pub const fn as_display_str(self) -> &'static str {
        match self {
            Self::Starting => Self::DISPLAY_STARTING,
            Self::Active => Self::DISPLAY_ACTIVE,
            Self::Draining => Self::DISPLAY_DRAINING,
            Self::Completed => Self::DISPLAY_COMPLETED,
            Self::Exited => Self::DISPLAY_EXITED,
            Self::Failed => Self::DISPLAY_FAILED,
        }
    }

    pub fn from_display_str(raw: &str) -> Option<Self> {
        match raw {
            Self::DISPLAY_STARTING => Some(Self::Starting),
            Self::DISPLAY_ACTIVE => Some(Self::Active),
            Self::DISPLAY_DRAINING => Some(Self::Draining),
            Self::DISPLAY_COMPLETED => Some(Self::Completed),
            Self::DISPLAY_EXITED => Some(Self::Exited),
            Self::DISPLAY_FAILED => Some(Self::Failed),
            _ => None,
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Exited | Self::Failed)
    }

    pub const fn is_active_or_draining(self) -> bool {
        matches!(self, Self::Starting | Self::Active | Self::Draining)
    }
}

impl fmt::Display for TraceLifecycleState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_display_str())
    }
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
    pub exited_at: Option<SystemTime>,
    pub failed_at: Option<SystemTime>,
}

impl TraceTiming {
    pub fn new(created_at: SystemTime) -> Self {
        Self {
            created_at,
            started_at: None,
            completed_at: None,
            exited_at: None,
            failed_at: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRecord {
    pub trace_id: TraceId,
    pub alert_token: TraceAlertToken,
    pub root_process_identity: ProcessIdentity,
    /// Readable, stable container id of the root process's container.
    /// `None` = host process or a runtime not resolved by the collector.
    /// 1:1 with `root_process_identity.pid_namespace`; resolved once at attach.
    pub root_container_id: Option<String>,
    /// Working directory captured from the root process during attach bootstrap.
    pub root_working_directory: Option<String>,
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
        alert_token: TraceAlertToken,
        root_process_identity: ProcessIdentity,
        display_name: TraceName,
        profile_name: ProfileName,
        created_at: SystemTime,
    ) -> Self {
        Self {
            trace_id,
            alert_token,
            root_process_identity,
            root_container_id: None,
            root_working_directory: None,
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

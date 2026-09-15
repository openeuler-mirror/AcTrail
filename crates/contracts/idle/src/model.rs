//! Core data models for idle detection.

use std::fmt;
use std::time::SystemTime;

use model_core::ids::TraceId;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdleIntervalId(u64);

impl IdleIntervalId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for IdleIntervalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "idle-interval-{}", self.0)
    }
}

/// Idle interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdleInterval {
    pub id: IdleIntervalId,
    pub trace_id: TraceId,
    pub task_id: String,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
}

impl IdleInterval {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() {
            return Err("task_id must not be empty");
        }
        if let Some(end_time) = self.end_time
            && end_time <= self.start_time
        {
            return Err("end_time must be strictly after start_time");
        }
        if self.start_time < SystemTime::UNIX_EPOCH {
            return Err("timestamps must not precede the unix epoch");
        }
        Ok(())
    }
}

/// Lifecycle transitions of one user task observed by the agent host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnLifecycleKind {
    Started,
    Completed,
}

impl TurnLifecycleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

/// Authoritative user-task lifecycle event produced by the agent host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnLifecycleEvent {
    pub trace_id: TraceId,
    pub task_id: String,
    pub kind: TurnLifecycleKind,
    pub observed_at: SystemTime,
}

impl TurnLifecycleEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() {
            return Err("task_id must not be empty");
        }
        if self.observed_at < SystemTime::UNIX_EPOCH {
            return Err("observed_at must not precede the unix epoch");
        }
        Ok(())
    }
}

/// State transition of one user interaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserInteractionState {
    Requested,
    Resolved,
}

impl UserInteractionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Resolved => "resolved",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "requested" => Some(Self::Requested),
            "resolved" => Some(Self::Resolved),
            _ => None,
        }
    }
}

/// Authoritative user-interaction event produced by the agent host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserInteractionEvent {
    pub trace_id: TraceId,
    pub task_id: String,
    pub interaction_id: String,
    pub state: UserInteractionState,
    pub observed_at: SystemTime,
}

impl UserInteractionEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() {
            return Err("task_id must not be empty");
        }
        if self.interaction_id.trim().is_empty() {
            return Err("interaction_id must not be empty");
        }
        if self.observed_at < SystemTime::UNIX_EPOCH {
            return Err("observed_at must not precede the unix epoch");
        }
        Ok(())
    }
}

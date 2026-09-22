//! Authoritative user lifecycle facts.

use model_core::ids::TraceId;
use std::time::SystemTime;

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
    pub session_id: String,
    pub task_id: String,
    pub kind: TurnLifecycleKind,
    pub observed_at: SystemTime,
}

impl TurnLifecycleEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() {
            return Err("session_id must not be empty");
        }
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
    pub session_id: String,
    pub task_id: String,
    pub interaction_id: String,
    pub state: UserInteractionState,
    pub observed_at: SystemTime,
}

impl UserInteractionEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() {
            return Err("session_id must not be empty");
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionClosedEvent {
    pub trace_id: TraceId,
    pub session_id: String,
    pub observed_at: SystemTime,
}

impl SessionClosedEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() {
            return Err("session_id must not be empty");
        }
        if self.observed_at < SystemTime::UNIX_EPOCH {
            return Err("observed_at must not precede the unix epoch");
        }
        Ok(())
    }
}

/// Model or tool work whose outstanding execution suspends hang detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkKind {
    Model,
    Tool,
}

impl WorkKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Tool => "tool",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "model" => Some(Self::Model),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkState {
    Started,
    Completed,
}

impl WorkState {
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

/// Exactly one start and completion per observed model or tool operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkLifecycleEvent {
    pub trace_id: TraceId,
    pub session_id: String,
    pub task_id: String,
    pub kind: WorkKind,
    pub state: WorkState,
    pub observed_at: SystemTime,
}

impl WorkLifecycleEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() {
            return Err("session_id must not be empty");
        }
        if self.task_id.trim().is_empty() {
            return Err("task_id must not be empty");
        }
        if self.observed_at < SystemTime::UNIX_EPOCH {
            return Err("observed_at must not precede the unix epoch");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserLifecycleEvent {
    Turn(TurnLifecycleEvent),
    Work(WorkLifecycleEvent),
    Interaction(UserInteractionEvent),
    SessionClosed(SessionClosedEvent),
}

impl UserLifecycleEvent {
    pub fn trace_id(&self) -> TraceId {
        match self {
            Self::Turn(event) => event.trace_id,
            Self::Work(event) => event.trace_id,
            Self::Interaction(event) => event.trace_id,
            Self::SessionClosed(event) => event.trace_id,
        }
    }
    pub fn session_id(&self) -> &str {
        match self {
            Self::Turn(event) => &event.session_id,
            Self::Work(event) => &event.session_id,
            Self::Interaction(event) => &event.session_id,
            Self::SessionClosed(event) => &event.session_id,
        }
    }
    pub fn observed_at(&self) -> SystemTime {
        match self {
            Self::Turn(event) => event.observed_at,
            Self::Work(event) => event.observed_at,
            Self::Interaction(event) => event.observed_at,
            Self::SessionClosed(event) => event.observed_at,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Turn(event) => event.validate(),
            Self::Work(event) => event.validate(),
            Self::Interaction(event) => event.validate(),
            Self::SessionClosed(event) => event.validate(),
        }
    }
}

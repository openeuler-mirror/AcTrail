//! File access enforcement configuration.

use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnforcementBackend {
    Fanotify,
}

impl FromStr for EnforcementBackend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fanotify" => Ok(Self::Fanotify),
            other => Err(format!("expected fanotify, got {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnforcementScope {
    Trace,
}

impl FromStr for EnforcementScope {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "trace" => Ok(Self::Trace),
            other => Err(format!("expected trace, got {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnforcementDecision {
    Allow,
    Deny,
}

impl EnforcementDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

impl FromStr for EnforcementDecision {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            other => Err(format!("expected allow or deny, got {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnforcementMarkStrategy {
    ParentDirectories,
}

impl FromStr for EnforcementMarkStrategy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "parent-directories" => Ok(Self::ParentDirectories),
            other => Err(format!("expected parent-directories, got {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnforcementConfig {
    pub enabled: bool,
    pub backend: EnforcementBackend,
    pub scope: EnforcementScope,
    pub rules_path: PathBuf,
    pub default_decision: EnforcementDecision,
    pub mark_strategy: EnforcementMarkStrategy,
    pub audit_enabled: bool,
    pub event_buffer_bytes: u32,
}

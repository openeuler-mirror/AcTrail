//! File access enforcement configuration.

use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EnforcementSeccompSyscall {
    Mkdir,
    Rmdir,
}

impl FromStr for EnforcementSeccompSyscall {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "mkdir" => Ok(Self::Mkdir),
            "rmdir" => Ok(Self::Rmdir),
            other => Err(format!("expected mkdir or rmdir, got {other}")),
        }
    }
}

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
    pub builtin_rules: Vec<EnforcementBuiltinRuleConfig>,
    pub default_decision: EnforcementDecision,
    pub mark_strategy: EnforcementMarkStrategy,
    pub audit_enabled: bool,
    pub event_buffer_bytes: u32,
    pub seccomp_syscalls: Vec<EnforcementSeccompSyscall>,
    pub seccomp_path_max_bytes: u32,
}

impl Default for EnforcementConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: EnforcementBackend::Fanotify,
            scope: EnforcementScope::Trace,
            rules_path: PathBuf::from("/etc/actrail/enforcement-rules.conf"),
            builtin_rules: Vec::new(),
            default_decision: EnforcementDecision::Allow,
            mark_strategy: EnforcementMarkStrategy::ParentDirectories,
            audit_enabled: true,
            event_buffer_bytes: 65_536,
            seccomp_syscalls: vec![
                EnforcementSeccompSyscall::Mkdir,
                EnforcementSeccompSyscall::Rmdir,
            ],
            seccomp_path_max_bytes: 4_096,
        }
    }
}

impl EnforcementConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnforcementBuiltinRuleConfig {
    pub rule_id: String,
    pub path: String,
}

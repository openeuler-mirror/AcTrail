//! Network-action control configuration.

use std::path::PathBuf;
use std::str::FromStr;

use super::EnforcementDecision;

pub const DEFAULT_NETWORK_PENDING_DECISION_MAX: u32 = 64;
pub const DEFAULT_NETWORK_REUSABLE_CACHE_MAX_ENTRIES: u32 = 4_096;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NetworkControlSeccompSyscall {
    Connect,
}

impl FromStr for NetworkControlSeccompSyscall {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "connect" => Ok(Self::Connect),
            other => Err(format!(
                "unsupported network control seccomp syscall {other}; supported: connect"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkControlConfig {
    pub enabled: bool,
    pub rules_path: PathBuf,
    pub syscalls: Vec<NetworkControlSeccompSyscall>,
    pub default_decision: EnforcementDecision,
    pub failure_decision: EnforcementDecision,
    pub audit_enabled: bool,
    pub audit_default_allow: bool,
    pub pending_decision_max: u32,
    pub reusable_cache_max_entries: u32,
}

impl Default for NetworkControlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rules_path: PathBuf::from("/etc/actrail/network-control.rules"),
            syscalls: vec![NetworkControlSeccompSyscall::Connect],
            default_decision: EnforcementDecision::Allow,
            failure_decision: EnforcementDecision::Deny,
            audit_enabled: true,
            audit_default_allow: false,
            pending_decision_max: DEFAULT_NETWORK_PENDING_DECISION_MAX,
            reusable_cache_max_entries: DEFAULT_NETWORK_REUSABLE_CACHE_MAX_ENTRIES,
        }
    }
}

impl NetworkControlConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

//! Command-execution control configuration.

use std::path::PathBuf;

use super::EnforcementDecision;

pub const DEFAULT_COMMAND_PATH_MAX_BYTES: u32 = 4_096;
pub const DEFAULT_COMMAND_ARGV_MAX_COUNT: u32 = 128;
pub const DEFAULT_COMMAND_ARGV_MAX_ARG_BYTES: u32 = 8_192;
pub const DEFAULT_COMMAND_ARGV_MAX_TOTAL_BYTES: u32 = 65_536;
pub const DEFAULT_COMMAND_PENDING_DECISION_MAX: u32 = 64;
pub const DEFAULT_COMMAND_REUSABLE_CACHE_MAX_ENTRIES: u32 = 4_096;
pub const DEFAULT_COMMAND_GRAY_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_COMMAND_GRAY_CONCURRENCY_LIMIT: u32 = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandControlGrayConfig {
    pub timeout_ms: u64,
    pub concurrency_limit: u32,
    pub fallback: EnforcementDecision,
}

impl Default for CommandControlGrayConfig {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_COMMAND_GRAY_TIMEOUT_MS,
            concurrency_limit: DEFAULT_COMMAND_GRAY_CONCURRENCY_LIMIT,
            fallback: EnforcementDecision::Deny,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandControlConfig {
    pub enabled: bool,
    pub rules_path: PathBuf,
    pub default_decision: EnforcementDecision,
    pub failure_decision: EnforcementDecision,
    pub audit_enabled: bool,
    pub audit_default_allow: bool,
    pub path_max_bytes: u32,
    pub argv_max_count: u32,
    pub argv_max_arg_bytes: u32,
    pub argv_max_total_bytes: u32,
    pub pending_decision_max: u32,
    pub reusable_cache_max_entries: u32,
    pub gray: CommandControlGrayConfig,
}

impl Default for CommandControlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rules_path: PathBuf::from("/etc/actrail/command-control.rules"),
            default_decision: EnforcementDecision::Allow,
            failure_decision: EnforcementDecision::Deny,
            audit_enabled: true,
            audit_default_allow: false,
            path_max_bytes: DEFAULT_COMMAND_PATH_MAX_BYTES,
            argv_max_count: DEFAULT_COMMAND_ARGV_MAX_COUNT,
            argv_max_arg_bytes: DEFAULT_COMMAND_ARGV_MAX_ARG_BYTES,
            argv_max_total_bytes: DEFAULT_COMMAND_ARGV_MAX_TOTAL_BYTES,
            pending_decision_max: DEFAULT_COMMAND_PENDING_DECISION_MAX,
            reusable_cache_max_entries: DEFAULT_COMMAND_REUSABLE_CACHE_MAX_ENTRIES,
            gray: CommandControlGrayConfig::default(),
        }
    }
}

impl CommandControlConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

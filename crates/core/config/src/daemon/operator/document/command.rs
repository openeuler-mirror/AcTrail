use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct CommandControlDocument {
    pub enabled: bool,
    pub rules_path: String,
    pub default_decision: String,
    pub failure_decision: String,
    pub audit_enabled: bool,
    pub audit_default_allow: bool,
    pub path_max_bytes: u32,
    pub argv_max_count: u32,
    pub argv_max_arg_bytes: u32,
    pub argv_max_total_bytes: u32,
    pub pending_decision_max: u32,
    pub reusable_cache_max_entries: u32,
    pub gray: CommandControlGrayDocument,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct CommandControlGrayDocument {
    pub timeout_ms: u64,
    pub concurrency_limit: u32,
    pub fallback: String,
}

impl Default for CommandControlDocument {
    fn default() -> Self {
        let config = CommandControlConfig::default();
        Self {
            enabled: config.enabled,
            rules_path: config.rules_path.display().to_string(),
            default_decision: config.default_decision.as_str().to_string(),
            failure_decision: config.failure_decision.as_str().to_string(),
            audit_enabled: config.audit_enabled,
            audit_default_allow: config.audit_default_allow,
            path_max_bytes: config.path_max_bytes,
            argv_max_count: config.argv_max_count,
            argv_max_arg_bytes: config.argv_max_arg_bytes,
            argv_max_total_bytes: config.argv_max_total_bytes,
            pending_decision_max: config.pending_decision_max,
            reusable_cache_max_entries: config.reusable_cache_max_entries,
            gray: CommandControlGrayDocument::default(),
        }
    }
}

impl Default for CommandControlGrayDocument {
    fn default() -> Self {
        let config = CommandControlGrayConfig::default();
        Self {
            timeout_ms: config.timeout_ms,
            concurrency_limit: config.concurrency_limit,
            fallback: config.fallback.as_str().to_string(),
        }
    }
}

impl CommandControlDocument {
    pub(super) fn to_config(&self) -> Result<CommandControlConfig, String> {
        Ok(CommandControlConfig {
            enabled: self.enabled,
            rules_path: PathBuf::from(&self.rules_path),
            default_decision: parse_value(
                "command_control.default_decision",
                &self.default_decision,
            )?,
            failure_decision: parse_value(
                "command_control.failure_decision",
                &self.failure_decision,
            )?,
            audit_enabled: self.audit_enabled,
            audit_default_allow: self.audit_default_allow,
            path_max_bytes: require_positive_u32(
                "command_control.path_max_bytes",
                self.path_max_bytes,
            )?,
            argv_max_count: require_positive_u32(
                "command_control.argv_max_count",
                self.argv_max_count,
            )?,
            argv_max_arg_bytes: require_positive_u32(
                "command_control.argv_max_arg_bytes",
                self.argv_max_arg_bytes,
            )?,
            argv_max_total_bytes: require_positive_u32(
                "command_control.argv_max_total_bytes",
                self.argv_max_total_bytes,
            )?,
            pending_decision_max: require_positive_u32(
                "command_control.pending_decision_max",
                self.pending_decision_max,
            )?,
            reusable_cache_max_entries: require_positive_u32(
                "command_control.reusable_cache_max_entries",
                self.reusable_cache_max_entries,
            )?,
            gray: CommandControlGrayConfig {
                timeout_ms: require_positive_u64(
                    "command_control.gray.timeout_ms",
                    self.gray.timeout_ms,
                )?,
                concurrency_limit: require_positive_u32(
                    "command_control.gray.concurrency_limit",
                    self.gray.concurrency_limit,
                )?,
                fallback: parse_value("command_control.gray.fallback", &self.gray.fallback)?,
            },
        })
    }
}

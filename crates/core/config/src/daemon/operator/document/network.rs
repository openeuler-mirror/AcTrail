use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct NetworkControlDocument {
    pub enabled: bool,
    pub rules_path: String,
    pub syscalls: Vec<String>,
    pub default_decision: String,
    pub failure_decision: String,
    pub audit_enabled: bool,
    pub audit_default_allow: bool,
    pub pending_decision_max: u32,
    pub reusable_cache_max_entries: u32,
}

impl Default for NetworkControlDocument {
    fn default() -> Self {
        let config = NetworkControlConfig::default();
        Self {
            enabled: config.enabled,
            rules_path: config.rules_path.display().to_string(),
            syscalls: config
                .syscalls
                .into_iter()
                .map(network_control_seccomp_syscall_as_str)
                .map(str::to_string)
                .collect(),
            default_decision: config.default_decision.as_str().to_string(),
            failure_decision: config.failure_decision.as_str().to_string(),
            audit_enabled: config.audit_enabled,
            audit_default_allow: config.audit_default_allow,
            pending_decision_max: config.pending_decision_max,
            reusable_cache_max_entries: config.reusable_cache_max_entries,
        }
    }
}

impl NetworkControlDocument {
    pub(super) fn to_config(&self) -> Result<NetworkControlConfig, String> {
        Ok(NetworkControlConfig {
            enabled: self.enabled,
            rules_path: PathBuf::from(&self.rules_path),
            syscalls: parse_values("network_control.syscalls", &self.syscalls)?,
            default_decision: parse_value(
                "network_control.default_decision",
                &self.default_decision,
            )?,
            failure_decision: parse_value(
                "network_control.failure_decision",
                &self.failure_decision,
            )?,
            audit_enabled: self.audit_enabled,
            audit_default_allow: self.audit_default_allow,
            pending_decision_max: require_positive_u32(
                "network_control.pending_decision_max",
                self.pending_decision_max,
            )?,
            reusable_cache_max_entries: require_positive_u32(
                "network_control.reusable_cache_max_entries",
                self.reusable_cache_max_entries,
            )?,
        })
    }
}

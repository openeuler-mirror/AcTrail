use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct NetworkControlDocument {
    pub enabled: bool,
    pub rules_path: String,
    pub syscalls: Vec<String>,
}

impl Default for NetworkControlDocument {
    fn default() -> Self {
        let config = NetworkControlConfig::disabled();
        Self {
            enabled: config.enabled,
            rules_path: config.rules_path.display().to_string(),
            syscalls: config
                .syscalls
                .into_iter()
                .map(network_control_seccomp_syscall_as_str)
                .map(str::to_string)
                .collect(),
        }
    }
}

impl NetworkControlDocument {
    pub(super) fn to_config(&self) -> Result<NetworkControlConfig, String> {
        Ok(NetworkControlConfig {
            enabled: self.enabled,
            rules_path: PathBuf::from(&self.rules_path),
            syscalls: parse_values("network_control.syscalls", &self.syscalls)?,
        })
    }
}

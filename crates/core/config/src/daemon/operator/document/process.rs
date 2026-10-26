use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SeccompNotifyDocument {
    pub enabled: bool,
    pub reserved_listener_fd: u32,
}

impl Default for SeccompNotifyDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            reserved_listener_fd: 253,
        }
    }
}

impl SeccompNotifyDocument {
    pub(super) fn to_config(&self) -> SeccompNotifyConfig {
        SeccompNotifyConfig {
            enabled: self.enabled,
            reserved_listener_fd: self.reserved_listener_fd,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ProcessSeccompDocument {
    pub enabled: bool,
    pub syscalls: Vec<String>,
    pub max_args: u32,
    pub max_arg_bytes: u32,
    pub max_total_arg_bytes: u32,
    /// Bounds process observations and semantic exec intents awaiting completion.
    pub pending_max_entries: u32,
}

impl Default for ProcessSeccompDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            syscalls: ["execve", "execveat"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            max_args: 128,
            max_arg_bytes: 4095,
            max_total_arg_bytes: 4095,
            pending_max_entries: 8192,
        }
    }
}

impl ProcessSeccompDocument {
    pub(super) fn to_config(&self) -> Result<ProcessSeccompConfig, String> {
        Ok(ProcessSeccompConfig {
            enabled: self.enabled,
            syscalls: parse_values("process_seccomp.syscalls", &self.syscalls)?,
            max_args: require_positive_u32("process_seccomp.max_args", self.max_args)?,
            max_arg_bytes: require_positive_u32(
                "process_seccomp.max_arg_bytes",
                self.max_arg_bytes,
            )?,
            max_total_arg_bytes: require_positive_u32(
                "process_seccomp.max_total_arg_bytes",
                self.max_total_arg_bytes,
            )?,
            pending_max_entries: require_positive_u32(
                "process_seccomp.pending_max_entries",
                self.pending_max_entries,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct AgentInvocationDocument {
    pub enabled: bool,
    pub commands: Vec<String>,
    pub tool_names: Vec<String>,
}

impl Default for AgentInvocationDocument {
    fn default() -> Self {
        let config = AgentInvocationConfig::default();
        Self {
            enabled: config.enabled,
            commands: config.commands,
            tool_names: config.tool_names,
        }
    }
}

impl AgentInvocationDocument {
    pub(super) fn to_config(&self) -> AgentInvocationConfig {
        AgentInvocationConfig {
            enabled: self.enabled,
            commands: self.commands.clone(),
            tool_names: self.tool_names.clone(),
        }
    }
}

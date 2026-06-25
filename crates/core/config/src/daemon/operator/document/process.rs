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
            enabled: true,
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
    pub pending_max_entries: u32,
}

impl Default for ProcessSeccompDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            syscalls: ["execve", "execveat", "fork", "vfork", "clone", "clone3"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            max_args: 128,
            max_arg_bytes: 8192,
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
}

impl Default for AgentInvocationDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            commands: Vec::new(),
        }
    }
}

impl AgentInvocationDocument {
    pub(super) fn to_config(&self) -> AgentInvocationConfig {
        AgentInvocationConfig {
            enabled: self.enabled,
            commands: self.commands.clone(),
        }
    }
}

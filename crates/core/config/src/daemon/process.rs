//! Process-control seccomp configuration.

use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeccompNotifyConfig {
    pub enabled: bool,
    pub reserved_listener_fd: u32,
}

impl Default for SeccompNotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reserved_listener_fd: 253,
        }
    }
}

impl SeccompNotifyConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProcessSeccompSyscall {
    Execve,
    Execveat,
    Fork,
    Vfork,
    Clone,
    Clone3,
}

impl FromStr for ProcessSeccompSyscall {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "execve" => Ok(Self::Execve),
            "execveat" => Ok(Self::Execveat),
            "fork" => Ok(Self::Fork),
            "vfork" => Ok(Self::Vfork),
            "clone" => Ok(Self::Clone),
            "clone3" => Ok(Self::Clone3),
            other => Err(format!("unsupported process seccomp syscall {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSeccompConfig {
    pub enabled: bool,
    pub syscalls: Vec<ProcessSeccompSyscall>,
    pub max_args: u32,
    pub max_arg_bytes: u32,
    pub pending_max_entries: u32,
}

impl Default for ProcessSeccompConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            syscalls: vec![
                ProcessSeccompSyscall::Execve,
                ProcessSeccompSyscall::Execveat,
                ProcessSeccompSyscall::Fork,
                ProcessSeccompSyscall::Vfork,
                ProcessSeccompSyscall::Clone,
                ProcessSeccompSyscall::Clone3,
            ],
            max_args: 128,
            max_arg_bytes: 8_192,
            pending_max_entries: 8_192,
        }
    }
}

impl ProcessSeccompConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

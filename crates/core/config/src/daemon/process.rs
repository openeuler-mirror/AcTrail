//! Process-control seccomp configuration.

use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeccompNotifyConfig {
    pub enabled: bool,
    pub reserved_listener_fd: u32,
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

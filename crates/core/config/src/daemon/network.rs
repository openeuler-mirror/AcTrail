//! Network-action control configuration.

use std::path::PathBuf;
use std::str::FromStr;

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
}

impl Default for NetworkControlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rules_path: PathBuf::from("/etc/actrail/network-control.rules"),
            syscalls: vec![NetworkControlSeccompSyscall::Connect],
        }
    }
}

impl NetworkControlConfig {
    pub fn disabled() -> Self {
        Self::default()
    }
}

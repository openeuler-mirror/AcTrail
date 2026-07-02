//! Command-execution control configuration.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandControlConfig {
    pub enabled: bool,
    pub rules_path: PathBuf,
}

impl Default for CommandControlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rules_path: PathBuf::from("/etc/actrail/command-control.rules"),
        }
    }
}

impl CommandControlConfig {
    pub fn disabled() -> Self {
        Self::default()
    }
}

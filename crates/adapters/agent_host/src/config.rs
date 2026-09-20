use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Configuration loaded explicitly by the launching application.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentHostConfig {
    pub(crate) opencode: OpenCodeConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct OpenCodeConfig {
    pub(crate) enabled: bool,
    pub(crate) plugin_dir: Option<PathBuf>,
}

impl AgentHostConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let document = std::fs::read_to_string(path)
            .map_err(|error| format!("read agent host config {}: {error}", path.display()))?;
        let mut config: Self = toml::from_str(&document)
            .map_err(|error| format!("parse agent host config {}: {error}", path.display()))?;
        if config.opencode.enabled && config.opencode.plugin_dir.is_none() {
            return Err("opencode.plugin_dir is required when opencode.enabled=true".to_string());
        }
        if let Some(directory) = &mut config.opencode.plugin_dir {
            if directory.as_os_str().is_empty() {
                return Err("opencode.plugin_dir must not be empty".to_string());
            }
            if directory.is_relative() {
                *directory = path.parent().unwrap_or(Path::new(".")).join(&*directory);
            }
        }
        Ok(config)
    }
}

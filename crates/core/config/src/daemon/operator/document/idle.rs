//! Parses and serializes idle-detection operator configuration.

use super::super::super::IdleDetectionConfig;
use super::helpers::{duration_as_string, parse_required_duration};
use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct IdleDetectionDocument {
    pub enabled: bool,
    pub opencode_auto_inject: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opencode_plugin_dir: Option<PathBuf>,
    pub threshold_secs: String,
}

impl Default for IdleDetectionDocument {
    fn default() -> Self {
        Self::from_config(&IdleDetectionConfig::default())
    }
}

impl IdleDetectionDocument {
    pub(super) fn from_config(config: &IdleDetectionConfig) -> Self {
        Self {
            enabled: config.enabled,
            opencode_auto_inject: config.opencode_auto_inject,
            opencode_plugin_dir: config.opencode_plugin_dir.clone(),
            threshold_secs: duration_as_string(config.threshold),
        }
    }

    pub(super) fn to_config(&self) -> Result<IdleDetectionConfig, String> {
        Ok(IdleDetectionConfig {
            enabled: self.enabled,
            opencode_auto_inject: self.opencode_auto_inject,
            opencode_plugin_dir: self.opencode_plugin_dir.clone(),
            threshold: parse_required_duration(
                "idle_detection.threshold_secs",
                &self.threshold_secs,
            )?,
        })
    }
}

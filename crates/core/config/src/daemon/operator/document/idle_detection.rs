//! Operator serialization of the idle detector's configuration.
use super::helpers::{duration_as_string, parse_required_duration};
use super::*;
use idle_detector::IdleDetectionConfig;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct IdleDetectionDocument {
    enabled: bool,
    threshold_secs: String,
    poll_interval_secs: String,
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
            threshold_secs: duration_as_string(config.threshold),
            poll_interval_secs: duration_as_string(config.poll_interval),
        }
    }

    pub(super) fn to_config(&self) -> Result<IdleDetectionConfig, String> {
        let config = IdleDetectionConfig {
            enabled: self.enabled,
            threshold: parse_required_duration(
                "idle_detection.threshold_secs",
                &self.threshold_secs,
            )?,
            poll_interval: parse_required_duration(
                "idle_detection.poll_interval_secs",
                &self.poll_interval_secs,
            )?,
        };
        config.validate()?;
        Ok(config)
    }
}

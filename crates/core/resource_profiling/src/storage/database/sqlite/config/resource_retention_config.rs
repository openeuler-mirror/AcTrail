use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceRetentionConfig {
    pub enabled: bool,
    pub max_age_seconds: u64,
    pub sweep_interval_seconds: u64,
    pub max_samples_per_sweep: u32,
}
impl Default for ResourceRetentionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_age_seconds: 604800,
            sweep_interval_seconds: 60,
            max_samples_per_sweep: 4096,
        }
    }
}
impl ResourceRetentionConfig {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.enabled
            && (self.max_age_seconds == 0
                || self.sweep_interval_seconds == 0
                || self.max_samples_per_sweep == 0)
        {
            return Err("resource SQLite retention values must be positive".into());
        }
        Ok(())
    }
}

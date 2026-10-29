use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedV1ScopeConfig {
    pub enabled: bool,
    pub root_name: String,
    pub recovery_scan_limit: u32,
    pub finalization_timeout_ms: u64,
}
impl Default for ManagedV1ScopeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root_name: "actrail".into(),
            recovery_scan_limit: 1024,
            finalization_timeout_ms: 30_000,
        }
    }
}
impl ManagedV1ScopeConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        const PREFIX: &str = "resource_profiling.collection.workload.cgroup.v1.managed";
        if self.root_name.is_empty()
            || matches!(self.root_name.as_str(), "." | "..")
            || !self
                .root_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(format!(
                "{PREFIX}.root_name must be a single directory name of [A-Za-z0-9_-]"
            ));
        }
        if self.recovery_scan_limit == 0 {
            return Err(format!("{PREFIX}.recovery_scan_limit must be positive"));
        }
        if self.finalization_timeout_ms == 0
            || std::time::Instant::now()
                .checked_add(std::time::Duration::from_millis(
                    self.finalization_timeout_ms,
                ))
                .is_none()
        {
            return Err(format!(
                "{PREFIX}.finalization_timeout_ms must be positive and representable"
            ));
        }
        Ok(())
    }
}

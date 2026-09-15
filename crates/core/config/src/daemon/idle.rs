//! Idle-detection runtime configuration.

use std::path::PathBuf;
use std::time::Duration;

/// Default no-observable-progress threshold when nothing is configured.
pub const DEFAULT_IDLE_DETECTION_THRESHOLD: Duration = Duration::from_secs(30);
/// Default installation directory for the OpenCode idle-detection adapter.
pub const DEFAULT_OPENCODE_PLUGIN_DIR: &str = "/usr/local/lib/actrail/opencode";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdleDetectionConfig {
    /// Whether the daemon records no-observable-progress intervals at all.
    pub enabled: bool,
    /// Whether `actrailctl launch -- opencode` injects the AcTrail OpenCode plugin.
    pub opencode_auto_inject: bool,
    /// Directory containing the OpenCode plugin and its supporting modules.
    pub opencode_plugin_dir: Option<PathBuf>,
    /// Silence duration after which an active task without progress opens an
    /// idle interval.
    pub threshold: Duration,
}

impl Default for IdleDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            opencode_auto_inject: false,
            opencode_plugin_dir: Some(PathBuf::from(DEFAULT_OPENCODE_PLUGIN_DIR)),
            threshold: DEFAULT_IDLE_DETECTION_THRESHOLD,
        }
    }
}

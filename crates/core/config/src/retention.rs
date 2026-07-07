//! Storage retention thresholds and sweep behavior.

use std::time::Duration;

pub const DEFAULT_STORAGE_RETENTION_ENABLED: bool = true;
pub const DEFAULT_STORAGE_RETENTION_MAX_TRACE_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const DEFAULT_STORAGE_RETENTION_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
pub const DEFAULT_STORAGE_RETENTION_MIN_TERMINAL_AGE: Duration = Duration::from_secs(30);
pub const DEFAULT_STORAGE_RETENTION_TRACES_PER_SWEEP: u32 = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageRetentionConfig {
    pub enabled: bool,
    pub max_trace_age: Duration,
    pub sweep_interval: Duration,
    pub min_terminal_age: Duration,
    pub max_traces_per_sweep: u32,
    pub protected_tags: Vec<String>,
    pub checkpoint_after_sweep: bool,
}

impl Default for StorageRetentionConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_STORAGE_RETENTION_ENABLED,
            max_trace_age: DEFAULT_STORAGE_RETENTION_MAX_TRACE_AGE,
            sweep_interval: DEFAULT_STORAGE_RETENTION_SWEEP_INTERVAL,
            min_terminal_age: DEFAULT_STORAGE_RETENTION_MIN_TERMINAL_AGE,
            max_traces_per_sweep: DEFAULT_STORAGE_RETENTION_TRACES_PER_SWEEP,
            protected_tags: vec!["retain".to_string(), "pinned".to_string()],
            checkpoint_after_sweep: true,
        }
    }
}

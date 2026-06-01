//! Retention thresholds and sweep behavior.

use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteCount {
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetentionConfig {
    pub max_trace_age: Duration,
    pub max_storage_bytes: ByteCount,
    pub sweep_interval: Duration,
    pub tombstone_retention: Duration,
}

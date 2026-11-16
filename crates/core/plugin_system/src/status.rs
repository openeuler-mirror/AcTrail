//! Runtime sources for plugin metrics.

use plugin_system_contracts::PluginHostcallMetrics;
use std::collections::BTreeMap;

pub trait PluginHostcallMetricsSource: Send + Sync {
    fn snapshot(&self) -> PluginHostcallMetrics;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginOperationalMetrics {
    pub queue_depth: Option<u64>,
    pub queue_capacity: Option<u32>,
    pub dropped_records: u64,
    pub last_error: Option<String>,
    pub values: BTreeMap<String, u64>,
}

pub trait PluginOperationalMetricsSource: Send + Sync {
    fn snapshot(&self) -> PluginOperationalMetrics;
}

//! Resource metric sampler configuration.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceMetricsConfig {
    pub enabled: bool,
    pub interval_ms: u64,
    pub include_children: bool,
    pub include_system: bool,
    pub cpu_alert_percent_millis: Option<u64>,
    pub memory_alert_rss_kb: Option<u64>,
}

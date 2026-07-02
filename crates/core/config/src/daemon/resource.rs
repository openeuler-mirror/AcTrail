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

impl Default for ResourceMetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_ms: 1_000,
            include_children: true,
            include_system: true,
            cpu_alert_percent_millis: None,
            memory_alert_rss_kb: None,
        }
    }
}

impl ResourceMetricsConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

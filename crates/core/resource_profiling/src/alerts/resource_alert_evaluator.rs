use super::ResourceAlertConfig;
use crate::model::ResourceMetrics;

pub struct ResourceAlertEvaluator {
    config: ResourceAlertConfig,
}

impl ResourceAlertEvaluator {
    pub fn new(config: ResourceAlertConfig) -> Self {
        Self { config }
    }

    pub fn evaluate(&self, payload: &mut ResourceMetrics) {
        let checks = [
            (
                "cpu_alert_percent_millis",
                self.config.cpu_alert_percent_millis,
                payload.cpu_percent_millis,
            ),
            (
                "memory_alert_rss_kb",
                self.config.memory_alert_rss_kb,
                payload.rss_kb,
            ),
            (
                "memory_alert_current_bytes",
                self.config.memory_alert_current_bytes,
                payload.memory_current_bytes,
            ),
        ];
        let mut alert = false;
        let mut missing = false;
        for (key, threshold, observed) in checks {
            if let Some(threshold) = threshold {
                payload.metadata.insert(key.into(), threshold.to_string());
                match observed {
                    Some(value) => alert |= value >= threshold,
                    None => missing = true,
                }
            }
        }
        payload.metadata.insert(
            "alert".into(),
            if alert {
                "true"
            } else if missing {
                "unknown"
            } else {
                "false"
            }
            .into(),
        );
    }
}

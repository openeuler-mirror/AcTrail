use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceAlertConfig {
    pub cpu_alert_percent_millis: Option<u64>,
    pub memory_alert_rss_kb: Option<u64>,
    pub memory_alert_current_bytes: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct AlertDocument {
    cpu_alert_percent_millis: String,
    memory_alert_rss_kb: String,
    memory_alert_current_bytes: String,
}

impl Default for AlertDocument {
    fn default() -> Self {
        Self {
            cpu_alert_percent_millis: "disabled".into(),
            memory_alert_rss_kb: "disabled".into(),
            memory_alert_current_bytes: "disabled".into(),
        }
    }
}

impl ResourceAlertConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: AlertDocument = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| format!("resource_profiling.alerts: {error}"))?;
        Ok(Self {
            cpu_alert_percent_millis: Self::threshold(
                "cpu_alert_percent_millis",
                &raw.cpu_alert_percent_millis,
            )?,
            memory_alert_rss_kb: Self::threshold("memory_alert_rss_kb", &raw.memory_alert_rss_kb)?,
            memory_alert_current_bytes: Self::threshold(
                "memory_alert_current_bytes",
                &raw.memory_alert_current_bytes,
            )?,
        })
    }

    pub fn default_document() -> Result<toml::Table, String> {
        toml::Table::try_from(AlertDocument::default()).map_err(|error| error.to_string())
    }

    fn threshold(key: &str, value: &str) -> Result<Option<u64>, String> {
        if value == "disabled" {
            return Ok(None);
        }
        value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| {
                format!("resource_profiling.alerts.{key} must be disabled or a positive integer")
            })
    }
}

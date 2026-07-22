use alert_contract::{AlertId, AlertListLimit, AlertView};
use model_core::ids::TraceId;
use storage_core::{StorageBackend, StorageOpenMode};
use storage_factory::{StorageConfig, open_storage_backend};

use crate::json;

pub(crate) struct AlertProjection {
    storage: Box<dyn StorageBackend>,
}

#[derive(Debug)]
pub(crate) enum AlertProjectionError {
    InvalidLimit(String),
    NotFound(String),
    Storage(String),
}

impl AlertProjection {
    pub(crate) fn open(storage_config: &StorageConfig) -> Result<Self, AlertProjectionError> {
        let storage =
            open_storage_backend(storage_config, StorageOpenMode::ReadOnly).map_err(|error| {
                AlertProjectionError::Storage(format!(
                    "open storage read-only failed: {}: {}",
                    error.stage, error.message
                ))
            })?;
        Ok(Self { storage })
    }

    pub(crate) fn latest_json(&self, limit: usize) -> Result<String, AlertProjectionError> {
        let limit = Self::limit(limit)?;
        let alerts = self
            .storage
            .latest_alerts(limit)
            .map_err(Self::store_error)?;
        self.collection_json(&alerts)
    }

    pub(crate) fn detail_json(&self, alert_id: u64) -> Result<String, AlertProjectionError> {
        let alert = self
            .storage
            .get_alert(AlertId::new(alert_id))
            .map_err(Self::store_error)?
            .ok_or_else(|| AlertProjectionError::NotFound(format!("alert {alert_id} not found")))?;
        Ok(format!("{{\"alert\":{}}}", self.render_alert(&alert)?))
    }

    pub(crate) fn trace_json(
        &self,
        trace_id: u64,
        limit: usize,
    ) -> Result<String, AlertProjectionError> {
        let trace_id = TraceId::new(trace_id);
        if self
            .storage
            .get_trace(trace_id)
            .map_err(Self::storage_error)?
            .is_none()
        {
            return Err(AlertProjectionError::NotFound(format!(
                "trace {trace_id} not found"
            )));
        }
        let alerts = self
            .storage
            .trace_alerts(trace_id, Self::limit(limit)?)
            .map_err(Self::store_error)?;
        self.collection_json(&alerts)
    }

    fn collection_json(&self, alerts: &[AlertView]) -> Result<String, AlertProjectionError> {
        let rows = alerts
            .iter()
            .map(|alert| self.render_alert(alert))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("{{\"alerts\":[{}]}}", rows.join(",")))
    }

    fn render_alert(&self, alert: &AlertView) -> Result<String, AlertProjectionError> {
        let payload = serde_json::from_str::<serde_json::Value>(&alert.record.payload_json)
            .map_err(|error| {
                AlertProjectionError::Storage(format!(
                    "stored alert {} contains invalid payload JSON: {error}",
                    alert.record.alert_id
                ))
            })?;
        let payload = serde_json::to_string(&payload).map_err(|error| {
            AlertProjectionError::Storage(format!(
                "render alert {} payload failed: {error}",
                alert.record.alert_id
            ))
        })?;
        let mut output = String::from("{");
        json::field(
            &mut output,
            "alert_id",
            &json::number(alert.record.alert_id.get()),
        );
        output.push(',');
        json::field(
            &mut output,
            "trace_id",
            &json::number(alert.record.trace_id.get()),
        );
        output.push(',');
        json::field(
            &mut output,
            "created_at",
            &json::time(alert.record.created_at),
        );
        output.push(',');
        json::field(
            &mut output,
            "producer_plugin_id",
            &json::string(&alert.definition.producer_plugin_id),
        );
        output.push(',');
        json::field(
            &mut output,
            "definition_key",
            &json::string(&alert.definition.definition_key),
        );
        output.push(',');
        json::field(&mut output, "kind", &json::string(&alert.definition.kind));
        output.push(',');
        json::field(&mut output, "title", &json::string(&alert.definition.title));
        output.push(',');
        json::field(
            &mut output,
            "severity",
            &json::string(alert.definition.severity.as_str()),
        );
        output.push(',');
        json::field(&mut output, "payload", &payload);
        output.push('}');
        Ok(output)
    }

    fn limit(limit: usize) -> Result<AlertListLimit, AlertProjectionError> {
        AlertListLimit::new(limit).ok_or_else(|| {
            AlertProjectionError::InvalidLimit("alert limit must be positive".to_string())
        })
    }

    fn store_error(error: alert_contract::AlertStoreError) -> AlertProjectionError {
        AlertProjectionError::Storage(format!("{}: {}", error.stage, error.message))
    }

    fn storage_error(error: storage_core::StorageError) -> AlertProjectionError {
        AlertProjectionError::Storage(format!("{}: {}", error.stage, error.message))
    }
}

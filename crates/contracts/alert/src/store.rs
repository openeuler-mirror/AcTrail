//! Storage Interfaces for normalized alert definitions and occurrences.

use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::trace::TraceAlertToken;

use crate::{
    AlertDefinition, AlertDefinitionId, AlertDraft, AlertId, AlertListLimit, AlertRecord,
    AlertStoreError, AlertSubmitOutcome, AlertView,
};

pub trait AlertDefinitionStore {
    /// Registers one shared output definition.
    ///
    /// Re-registering identical metadata returns the existing identifier. Reusing
    /// `(producer_plugin_id, definition_key)` with different metadata fails with
    /// `DefinitionConflict`.
    fn register_alert_definition(
        &mut self,
        definition: &AlertDefinition,
    ) -> Result<AlertDefinitionId, AlertStoreError>;

    fn get_alert_definition(
        &self,
        definition_id: AlertDefinitionId,
    ) -> Result<Option<AlertDefinition>, AlertStoreError>;
}

pub trait AlertWriteStore {
    /// Durably appends one alert occurrence for the current trace and producer.
    ///
    /// A matching trace/token pair creates a distinct occurrence. Missing traces
    /// and mismatched tokens are rejected without writing an occurrence.
    fn submit_alert(
        &mut self,
        trace_id: TraceId,
        alert_token: &TraceAlertToken,
        producer_plugin_id: &str,
        draft: &AlertDraft,
        created_at: SystemTime,
    ) -> Result<AlertSubmitOutcome, AlertStoreError>;
}

pub trait AlertReadStore {
    fn latest_alerts(&self, limit: AlertListLimit) -> Result<Vec<AlertView>, AlertStoreError>;

    fn get_alert(&self, alert_id: AlertId) -> Result<Option<AlertView>, AlertStoreError>;

    fn trace_alerts(
        &self,
        trace_id: TraceId,
        limit: AlertListLimit,
    ) -> Result<Vec<AlertView>, AlertStoreError>;

    fn get_alert_record(&self, alert_id: AlertId) -> Result<Option<AlertRecord>, AlertStoreError> {
        self.get_alert(alert_id)
            .map(|value| value.map(|view| view.record))
    }
}

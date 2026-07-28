use std::time::SystemTime;

use alert_contract::{
    AlertDefinitionId, AlertDraft, AlertId, AlertStoreError, AlertStoreErrorKind,
    AlertSubmitOutcome, AlertWriteStore,
};
use model_core::ids::TraceId;
use model_core::trace::TraceAlertToken;
use rusqlite::{OptionalExtension, params};

use super::codec::AlertInputValidator;
use crate::SqliteStorage;
use crate::records::encode_time;

impl AlertWriteStore for SqliteStorage {
    fn submit_alert(
        &mut self,
        trace_id: TraceId,
        alert_token: &TraceAlertToken,
        producer_plugin_id: &str,
        draft: &AlertDraft,
        created_at: SystemTime,
    ) -> Result<AlertSubmitOutcome, AlertStoreError> {
        if producer_plugin_id.trim().is_empty() {
            return Err(AlertStoreError::new(
                AlertStoreErrorKind::InvalidDefinition,
                "submit_alert",
                "producer_plugin_id must not be empty",
            ));
        }
        let mut connection = self.connection().borrow_mut();
        let transaction = connection.transaction().map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "begin_alert_submit",
                error.to_string(),
            )
        })?;
        let stored_token = transaction
            .query_row(
                "SELECT alert_token FROM trace_alert_authorizations WHERE trace_id = ?1",
                params![trace_id.get()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "resolve_alert_authorization",
                    error.to_string(),
                )
            })?;
        let Some(stored_token) = stored_token else {
            return Ok(AlertSubmitOutcome::RejectedTraceToken);
        };
        let stored_token = TraceAlertToken::from_slice(&stored_token).ok_or_else(|| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "resolve_alert_authorization",
                "stored trace alert token has an invalid length",
            )
        })?;
        if &stored_token != alert_token {
            return Ok(AlertSubmitOutcome::RejectedTraceToken);
        }
        let canonical_payload = AlertInputValidator::canonical_payload(draft)?;
        let definition_id = transaction
            .query_row(
                "SELECT alert_definition_id FROM alert_definitions
                 WHERE producer_plugin_id = ?1 AND definition_key = ?2",
                params![producer_plugin_id, draft.definition_key],
                |row| row.get::<_, u64>(0).map(AlertDefinitionId::new),
            )
            .optional()
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "resolve_alert_definition",
                    error.to_string(),
                )
            })?
            .ok_or_else(|| {
                AlertStoreError::new(
                    AlertStoreErrorKind::NotFound,
                    "resolve_alert_definition",
                    "alert definition is not registered for this producer",
                )
            })?;
        transaction
            .execute(
                "INSERT INTO alerts (
                    trace_id, alert_definition_id, created_at, payload_json
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    trace_id.get(),
                    definition_id.get(),
                    encode_time(created_at),
                    canonical_payload,
                ],
            )
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "insert_alert",
                    error.to_string(),
                )
            })?;
        let alert_id = u64::try_from(transaction.last_insert_rowid()).map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "allocate_alert_id",
                error.to_string(),
            )
        })?;
        transaction.commit().map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "commit_alert_submit",
                error.to_string(),
            )
        })?;
        Ok(AlertSubmitOutcome::Stored(AlertId::new(alert_id)))
    }
}

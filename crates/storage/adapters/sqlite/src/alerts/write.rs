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
        if draft
            .deduplication_key
            .as_ref()
            .is_some_and(|key| key.trim().is_empty() || key.len() > 256)
        {
            return Err(AlertStoreError::new(
                AlertStoreErrorKind::InvalidPayload,
                "validate_alert_deduplication",
                "alert deduplication key must contain 1 to 256 bytes",
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
        let canonical_payload = patch_payload_timestamp(&canonical_payload, created_at);
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
        if let Some(deduplication_key) = &draft.deduplication_key {
            let inserted = transaction
                .execute(
                    "INSERT OR IGNORE INTO alert_deduplication_keys (
                        trace_id, alert_definition_id, deduplication_key
                     ) VALUES (?1, ?2, ?3)",
                    params![trace_id.get(), definition_id.get(), deduplication_key],
                )
                .map_err(|error| {
                    AlertStoreError::new(
                        AlertStoreErrorKind::StorageFailure,
                        "reserve_alert_deduplication_key",
                        error.to_string(),
                    )
                })?;
            if inserted == 0 {
                return Ok(AlertSubmitOutcome::DuplicateSuppressed);
            }
        }
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

fn patch_payload_timestamp(payload: &str, now: SystemTime) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return payload.to_string();
    };
    let Some(obj) = value.as_object_mut() else {
        return payload.to_string();
    };
    let epoch_secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ts = epoch_secs_to_iso8601(epoch_secs);
    obj.insert("timestamp".to_string(), serde_json::Value::String(ts));
    serde_json::to_string(&obj).unwrap_or_else(|_| payload.to_string())
}

fn epoch_secs_to_iso8601(secs: u64) -> String {
    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let total_days = (secs / 86400) as u32;
    let rem = (secs % 86400) as u32;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let mut days_left = total_days;
    let mut year = 1970u32;
    loop {
        let diy = if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
            366
        } else {
            365
        };
        if days_left < diy {
            break;
        }
        days_left -= diy;
        year += 1;
    }
    let mut month = 1u32;
    for m_idx in 0..12u32 {
        month = m_idx + 1;
        let dim = if m_idx == 1 && ((year % 4 == 0 && year % 100 != 0) || year % 400 == 0) {
            29
        } else {
            DAYS_IN_MONTH[m_idx as usize]
        };
        if days_left < dim {
            break;
        }
        days_left -= dim;
    }
    let day = days_left + 1;
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

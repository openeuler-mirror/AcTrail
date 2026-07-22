use alert_contract::{
    AlertDefinition, AlertDefinitionId, AlertDraft, AlertId, AlertRecord, AlertSeverity,
    AlertStoreError, AlertStoreErrorKind, AlertView,
};
use model_core::ids::TraceId;
use rusqlite::Row;

use crate::records::decode_time;

pub(super) struct AlertRowCodec;

impl AlertRowCodec {
    pub(super) fn severity_code(severity: AlertSeverity) -> i64 {
        match severity {
            AlertSeverity::Informational => 1,
            AlertSeverity::Low => 2,
            AlertSeverity::Medium => 3,
            AlertSeverity::High => 4,
            AlertSeverity::Critical => 5,
        }
    }

    fn severity_from_code(code: i64) -> Result<AlertSeverity, rusqlite::Error> {
        match code {
            1 => Ok(AlertSeverity::Informational),
            2 => Ok(AlertSeverity::Low),
            3 => Ok(AlertSeverity::Medium),
            4 => Ok(AlertSeverity::High),
            5 => Ok(AlertSeverity::Critical),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }

    pub(super) fn definition_from_row(
        row: &Row<'_>,
        offset: usize,
    ) -> Result<AlertDefinition, rusqlite::Error> {
        Ok(AlertDefinition {
            producer_plugin_id: row.get(offset)?,
            definition_key: row.get(offset + 1)?,
            kind: row.get(offset + 2)?,
            title: row.get(offset + 3)?,
            severity: Self::severity_from_code(row.get(offset + 4)?)?,
            payload_schema_id: row.get(offset + 5)?,
        })
    }

    pub(super) fn view_from_row(row: &Row<'_>) -> Result<AlertView, rusqlite::Error> {
        Ok(AlertView {
            record: AlertRecord {
                alert_id: AlertId::new(row.get(0)?),
                trace_id: TraceId::new(row.get(1)?),
                alert_definition_id: AlertDefinitionId::new(row.get(2)?),
                created_at: decode_time(row.get(3)?),
                payload_json: row.get(4)?,
            },
            definition: Self::definition_from_row(row, 5)?,
        })
    }
}

pub(super) struct AlertInputValidator;

impl AlertInputValidator {
    pub(super) fn definition(definition: &AlertDefinition) -> Result<(), AlertStoreError> {
        definition.validate().map_err(|message| {
            AlertStoreError::new(
                AlertStoreErrorKind::InvalidDefinition,
                "validate_alert_definition",
                message,
            )
        })
    }

    pub(super) fn canonical_payload(draft: &AlertDraft) -> Result<String, AlertStoreError> {
        if draft.definition_key.trim().is_empty() {
            return Err(AlertStoreError::new(
                AlertStoreErrorKind::InvalidPayload,
                "validate_alert_draft",
                "definition_key must not be empty",
            ));
        }
        let value =
            serde_json::from_str::<serde_json::Value>(&draft.payload_json).map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::InvalidPayload,
                    "parse_alert_payload",
                    error.to_string(),
                )
            })?;
        if !value.is_object() {
            return Err(AlertStoreError::new(
                AlertStoreErrorKind::InvalidPayload,
                "validate_alert_payload",
                "payload_json must encode a JSON object",
            ));
        }
        serde_json::to_string(&value).map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::InvalidPayload,
                "canonicalize_alert_payload",
                error.to_string(),
            )
        })
    }
}

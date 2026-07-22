use alert_contract::{
    AlertDefinition, AlertDefinitionId, AlertDefinitionStore, AlertStoreError, AlertStoreErrorKind,
};
use rusqlite::{OptionalExtension, params};

use super::codec::{AlertInputValidator, AlertRowCodec};
use crate::SqliteStorage;

impl AlertDefinitionStore for SqliteStorage {
    fn register_alert_definition(
        &mut self,
        definition: &AlertDefinition,
    ) -> Result<AlertDefinitionId, AlertStoreError> {
        AlertInputValidator::definition(definition)?;
        let mut connection = self.connection().borrow_mut();
        let transaction = connection.transaction().map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "begin_alert_definition_registration",
                error.to_string(),
            )
        })?;
        let existing = transaction
            .query_row(
                "SELECT alert_definition_id, producer_plugin_id, definition_key, kind, title,
                        severity_code, payload_schema_id
                 FROM alert_definitions
                 WHERE producer_plugin_id = ?1 AND definition_key = ?2",
                params![definition.producer_plugin_id, definition.definition_key],
                |row| {
                    Ok((
                        AlertDefinitionId::new(row.get(0)?),
                        AlertRowCodec::definition_from_row(row, 1)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "query_alert_definition",
                    error.to_string(),
                )
            })?;
        if let Some((definition_id, stored)) = existing {
            if stored == *definition {
                transaction.commit().map_err(|error| {
                    AlertStoreError::new(
                        AlertStoreErrorKind::StorageFailure,
                        "commit_alert_definition_registration",
                        error.to_string(),
                    )
                })?;
                return Ok(definition_id);
            }
            return Err(AlertStoreError::new(
                AlertStoreErrorKind::DefinitionConflict,
                "register_alert_definition",
                "producer_plugin_id and definition_key already identify different metadata",
            ));
        }
        transaction
            .execute(
                "INSERT INTO alert_definitions (
                    producer_plugin_id, definition_key, kind, title, severity_code,
                    payload_schema_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    definition.producer_plugin_id,
                    definition.definition_key,
                    definition.kind,
                    definition.title,
                    AlertRowCodec::severity_code(definition.severity),
                    definition.payload_schema_id,
                ],
            )
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "insert_alert_definition",
                    error.to_string(),
                )
            })?;
        let definition_id = u64::try_from(transaction.last_insert_rowid()).map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "allocate_alert_definition_id",
                error.to_string(),
            )
        })?;
        transaction.commit().map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "commit_alert_definition_registration",
                error.to_string(),
            )
        })?;
        Ok(AlertDefinitionId::new(definition_id))
    }

    fn get_alert_definition(
        &self,
        definition_id: AlertDefinitionId,
    ) -> Result<Option<AlertDefinition>, AlertStoreError> {
        self.connection()
            .borrow()
            .query_row(
                "SELECT producer_plugin_id, definition_key, kind, title, severity_code,
                        payload_schema_id
                 FROM alert_definitions WHERE alert_definition_id = ?1",
                params![definition_id.get()],
                |row| AlertRowCodec::definition_from_row(row, 0),
            )
            .optional()
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "get_alert_definition",
                    error.to_string(),
                )
            })
    }
}

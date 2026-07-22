use alert_contract::{
    AlertId, AlertListLimit, AlertReadStore, AlertStoreError, AlertStoreErrorKind, AlertView,
};
use model_core::ids::TraceId;
use rusqlite::{OptionalExtension, params};

use super::codec::AlertRowCodec;
use crate::SqliteStorage;

impl AlertReadStore for SqliteStorage {
    fn latest_alerts(&self, limit: AlertListLimit) -> Result<Vec<AlertView>, AlertStoreError> {
        self.read_alert_views(
            "SELECT a.alert_id, a.trace_id, a.alert_definition_id, a.created_at, a.payload_json,
                    d.producer_plugin_id, d.definition_key, d.kind, d.title, d.severity_code,
                    d.payload_schema_id
             FROM alerts a
             JOIN alert_definitions d
               ON d.alert_definition_id = a.alert_definition_id
             ORDER BY a.created_at DESC, a.alert_id DESC
             LIMIT ?1",
            params![limit.get()],
            "latest_alerts",
        )
    }

    fn get_alert(&self, alert_id: AlertId) -> Result<Option<AlertView>, AlertStoreError> {
        self.connection()
            .borrow()
            .query_row(
                "SELECT a.alert_id, a.trace_id, a.alert_definition_id, a.created_at, a.payload_json,
                        d.producer_plugin_id, d.definition_key, d.kind, d.title, d.severity_code,
                        d.payload_schema_id
                 FROM alerts a
                 JOIN alert_definitions d
                   ON d.alert_definition_id = a.alert_definition_id
                 WHERE a.alert_id = ?1",
                params![alert_id.get()],
                AlertRowCodec::view_from_row,
            )
            .optional()
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "get_alert",
                    error.to_string(),
                )
            })
    }

    fn trace_alerts(
        &self,
        trace_id: TraceId,
        limit: AlertListLimit,
    ) -> Result<Vec<AlertView>, AlertStoreError> {
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare(
                "SELECT a.alert_id, a.trace_id, a.alert_definition_id, a.created_at, a.payload_json,
                        d.producer_plugin_id, d.definition_key, d.kind, d.title, d.severity_code,
                        d.payload_schema_id
                 FROM alerts a
                 JOIN alert_definitions d
                   ON d.alert_definition_id = a.alert_definition_id
                 WHERE a.trace_id = ?1
                 ORDER BY a.created_at DESC, a.alert_id DESC
                 LIMIT ?2",
            )
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "prepare_trace_alerts",
                    error.to_string(),
                )
            })?;
        let rows = statement
            .query_map(
                params![trace_id.get(), limit.get()],
                AlertRowCodec::view_from_row,
            )
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    "query_trace_alerts",
                    error.to_string(),
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "map_trace_alerts",
                error.to_string(),
            )
        })
    }
}

impl SqliteStorage {
    fn read_alert_views<P>(
        &self,
        sql: &str,
        parameters: P,
        stage: &'static str,
    ) -> Result<Vec<AlertView>, AlertStoreError>
    where
        P: rusqlite::Params,
    {
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(sql).map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                format!("prepare_{stage}"),
                error.to_string(),
            )
        })?;
        let rows = statement
            .query_map(parameters, AlertRowCodec::view_from_row)
            .map_err(|error| {
                AlertStoreError::new(
                    AlertStoreErrorKind::StorageFailure,
                    format!("query_{stage}"),
                    error.to_string(),
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                format!("map_{stage}"),
                error.to_string(),
            )
        })
    }
}

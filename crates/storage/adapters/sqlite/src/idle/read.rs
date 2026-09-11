//! Reads idle records from SQLite.

use idle_contract::{IdleInterval, IdleReadStore, IdleStoreError, IdleStoreErrorKind};
use model_core::ids::TraceId;
use rusqlite::params;

use super::codec::IdleRowCodec;
use crate::SqliteStorage;

const IDLE_INTERVAL_COLUMNS: &str = "interval_id, trace_id, task_id, start_time, end_time";

impl IdleReadStore for SqliteStorage {
    fn idle_intervals_for_trace(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<IdleInterval>, IdleStoreError> {
        self.read_idle_intervals(
            &format!(
                "SELECT {IDLE_INTERVAL_COLUMNS}
                 FROM idle_intervals WHERE trace_id = ?1
                 ORDER BY start_time, interval_id"
            ),
            params![trace_id.get()],
            "idle_intervals_for_trace",
        )
    }
}

impl SqliteStorage {
    fn read_idle_intervals<P>(
        &self,
        sql: &str,
        parameters: P,
        stage: &'static str,
    ) -> Result<Vec<IdleInterval>, IdleStoreError>
    where
        P: rusqlite::Params,
    {
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(sql).map_err(|error| {
            IdleStoreError::new(
                IdleStoreErrorKind::StorageFailure,
                format!("prepare_{stage}"),
                error.to_string(),
            )
        })?;
        let rows = statement
            .query_map(parameters, IdleRowCodec::interval_from_row)
            .map_err(|error| {
                IdleStoreError::new(
                    IdleStoreErrorKind::StorageFailure,
                    format!("query_{stage}"),
                    error.to_string(),
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            IdleStoreError::new(
                IdleStoreErrorKind::StorageFailure,
                format!("map_{stage}"),
                error.to_string(),
            )
        })
    }
}

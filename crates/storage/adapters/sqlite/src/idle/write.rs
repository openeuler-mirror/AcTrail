//! Writes idle records to SQLite.

use idle_contract::{
    IdleInterval, IdleIntervalId, IdleStoreError, IdleStoreErrorKind, IdleStoreOp, IdleWriteStore,
};
use rusqlite::params;

use super::codec::IdleInputValidator;
use crate::SqliteStorage;
use crate::records::encode_time;

impl IdleWriteStore for SqliteStorage {
    fn apply_idle_ops(&mut self, ops: &[IdleStoreOp]) -> Result<(), IdleStoreError> {
        if ops.is_empty() {
            return Ok(());
        }
        let owns_transaction = self.connection().borrow().is_autocommit();
        if owns_transaction {
            self.connection()
                .borrow_mut()
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|error| {
                    IdleStoreError::new(
                        IdleStoreErrorKind::StorageFailure,
                        "begin_idle_ops",
                        error.to_string(),
                    )
                })?;
        }

        for op in ops {
            let result = match op {
                IdleStoreOp::UpsertInterval(interval) => self.upsert_idle_interval(interval),
                IdleStoreOp::DeleteInterval(interval_id) => self.delete_idle_interval(*interval_id),
            };
            if let Err(error) = result {
                if owns_transaction {
                    let _ = self.connection().borrow_mut().execute_batch("ROLLBACK");
                }
                return Err(error);
            }
        }

        if !owns_transaction {
            return Ok(());
        }
        self.connection()
            .borrow_mut()
            .execute_batch("COMMIT")
            .map_err(|error| {
                let _ = self.connection().borrow_mut().execute_batch("ROLLBACK");
                IdleStoreError::new(
                    IdleStoreErrorKind::StorageFailure,
                    "commit_idle_ops",
                    error.to_string(),
                )
            })
    }
}

impl SqliteStorage {
    fn upsert_idle_interval(&mut self, interval: &IdleInterval) -> Result<(), IdleStoreError> {
        IdleInputValidator::interval(interval)?;
        self.connection()
            .borrow_mut()
            .execute(
                "INSERT INTO idle_intervals (
                    interval_id, trace_id, task_id, start_time, end_time
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(interval_id) DO UPDATE SET
                    trace_id = excluded.trace_id,
                    task_id = excluded.task_id,
                    start_time = excluded.start_time,
                    end_time = excluded.end_time",
                params![
                    interval.id.get(),
                    interval.trace_id.get(),
                    interval.task_id,
                    encode_time(interval.start_time),
                    interval.end_time.map(encode_time),
                ],
            )
            .map_err(|error| {
                IdleStoreError::new(
                    IdleStoreErrorKind::StorageFailure,
                    "upsert_idle_interval",
                    error.to_string(),
                )
            })?;
        Ok(())
    }

    fn delete_idle_interval(&mut self, interval_id: IdleIntervalId) -> Result<(), IdleStoreError> {
        self.connection()
            .borrow_mut()
            .execute(
                "DELETE FROM idle_intervals WHERE interval_id = ?1",
                params![interval_id.get()],
            )
            .map_err(|error| {
                IdleStoreError::new(
                    IdleStoreErrorKind::StorageFailure,
                    "delete_idle_interval",
                    error.to_string(),
                )
            })?;
        Ok(())
    }
}

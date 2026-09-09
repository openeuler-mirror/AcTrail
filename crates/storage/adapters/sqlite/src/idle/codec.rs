//! Encodes and validates idle records for SQLite.

use idle_contract::{IdleInterval, IdleIntervalId, IdleStoreError, IdleStoreErrorKind};
use model_core::ids::TraceId;
use rusqlite::Row;

use crate::records::decode_time;

pub(super) struct IdleRowCodec;

impl IdleRowCodec {
    pub(super) fn interval_from_row(row: &Row<'_>) -> Result<IdleInterval, rusqlite::Error> {
        Ok(IdleInterval {
            id: IdleIntervalId::new(row.get(0)?),
            trace_id: TraceId::new(row.get(1)?),
            task_id: row.get(2)?,
            start_time: decode_time(row.get(3)?),
            end_time: row.get::<_, Option<i64>>(4)?.map(decode_time),
        })
    }
}

pub(super) struct IdleInputValidator;

impl IdleInputValidator {
    pub(super) fn interval(interval: &IdleInterval) -> Result<(), IdleStoreError> {
        interval.validate().map_err(|message| {
            IdleStoreError::new(
                IdleStoreErrorKind::InvalidInterval,
                "validate_idle_interval",
                message,
            )
        })
    }
}

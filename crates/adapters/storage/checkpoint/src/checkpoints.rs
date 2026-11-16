//! Checkpoints, the backend snapshot each one refers to, invalidation and deletion.

use std::time::SystemTime;

use checkpoint_core::model::{
    CheckpointId, CheckpointRecord, Deletion, Failure, FilterMode, NewCheckpoint, OperationId,
    Release,
};
use model_core::ids::TraceId;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::codec::{encode_time, json, optional_time, time, to_json};

const SELECT_COLUMNS: &str = "SELECT checkpoint_id, trace_id, operation_id, created_at_ns,
    invalidated_by, deleted_at_ns, release_state, release_failure_json, snapshot_id
    FROM checkpoints";

pub(crate) fn insert(
    connection: &Connection,
    trace_id: TraceId,
    operation_id: OperationId,
    created_at: SystemTime,
    checkpoint: &NewCheckpoint,
) -> rusqlite::Result<CheckpointId> {
    let checkpoint_id = CheckpointId::new(connection.query_row(
        "SELECT COALESCE(MAX(checkpoint_id) + 1, 0) FROM checkpoints WHERE trace_id = ?1",
        [trace_id.get()],
        |row| row.get(0),
    )?);
    connection.execute(
        "INSERT INTO checkpoints
            (trace_id, checkpoint_id, operation_id, created_at_ns, snapshot_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            trace_id.get(),
            checkpoint_id.get(),
            operation_id.get(),
            encode_time(created_at),
            checkpoint.snapshot_id,
        ],
    )?;
    Ok(checkpoint_id)
}

/// Marks the checkpoints of the snapshots as invalidated by the restore and returns them.
pub(crate) fn invalidate(
    connection: &Connection,
    trace_id: TraceId,
    restore: OperationId,
    snapshots: &[Vec<u8>],
) -> rusqlite::Result<Vec<CheckpointId>> {
    let mut statement = connection.prepare_cached(
        "UPDATE checkpoints SET invalidated_by = ?1
         WHERE trace_id = ?2 AND snapshot_id = ?3 AND invalidated_by IS NULL
         RETURNING checkpoint_id",
    )?;
    let mut invalidated = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let checkpoint = statement
            .query_row(params![restore.get(), trace_id.get(), snapshot], |row| {
                Ok(CheckpointId::new(row.get(0)?))
            })
            .optional()?;
        invalidated.extend(checkpoint);
    }
    Ok(invalidated)
}

/// Returns the snapshot ID, or `None` for an unknown checkpoint. Deleting again keeps the first
/// time and asks for another release.
pub(crate) fn mark_deleted(
    connection: &Connection,
    trace_id: TraceId,
    checkpoint_id: CheckpointId,
    deleted_at: SystemTime,
) -> rusqlite::Result<Option<Vec<u8>>> {
    connection
        .query_row(
            "UPDATE checkpoints SET deleted_at_ns = COALESCE(deleted_at_ns, ?3),
                release_state = 'pending', release_failure_json = NULL
             WHERE trace_id = ?1 AND checkpoint_id = ?2
             RETURNING snapshot_id",
            params![trace_id.get(), checkpoint_id.get(), encode_time(deleted_at)],
            |row| row.get(0),
        )
        .optional()
}

pub(crate) fn record_release(
    connection: &Connection,
    trace_id: TraceId,
    checkpoint_id: CheckpointId,
    failure: Option<&Failure>,
) -> rusqlite::Result<usize> {
    let state = if failure.is_some() {
        "failed"
    } else {
        "released"
    };
    connection.execute(
        "UPDATE checkpoints SET release_state = ?3, release_failure_json = ?4
         WHERE trace_id = ?1 AND checkpoint_id = ?2 AND deleted_at_ns IS NOT NULL",
        params![
            trace_id.get(),
            checkpoint_id.get(),
            state,
            failure.map(to_json)
        ],
    )
}

pub(crate) fn get(
    connection: &Connection,
    trace_id: TraceId,
    checkpoint_id: CheckpointId,
) -> rusqlite::Result<Option<CheckpointRecord>> {
    connection
        .query_row(
            &format!("{SELECT_COLUMNS} WHERE trace_id = ?1 AND checkpoint_id = ?2"),
            [trace_id.get(), checkpoint_id.get()],
            from_row,
        )
        .optional()
}

pub(crate) fn for_trace(
    connection: &Connection,
    trace_id: TraceId,
) -> rusqlite::Result<Vec<CheckpointRecord>> {
    let mut statement = connection.prepare_cached(&format!(
        "{SELECT_COLUMNS} WHERE trace_id = ?1 ORDER BY checkpoint_id"
    ))?;
    let rows = statement.query_map([trace_id.get()], from_row)?;
    rows.collect()
}

/// The checkpoints `filter_mode` selects, oldest first. A negative SQLite limit means no limit.
pub(crate) fn matching(
    connection: &Connection,
    trace_id: TraceId,
    filter_mode: &FilterMode,
) -> rusqlite::Result<Vec<CheckpointRecord>> {
    let (since, until, limit) = match *filter_mode {
        FilterMode::All => (None, None, -1),
        FilterMode::Last(count) => (None, None, i64::from(count)),
        FilterMode::Created { since, until } => (since, until, -1),
    };
    let mut statement = connection.prepare_cached(&format!(
        "SELECT * FROM ({SELECT_COLUMNS}
            WHERE trace_id = ?1
              AND (?2 IS NULL OR created_at_ns >= ?2)
              AND (?3 IS NULL OR created_at_ns < ?3)
            ORDER BY checkpoint_id DESC LIMIT ?4)
         ORDER BY checkpoint_id"
    ))?;
    let rows = statement.query_map(
        params![
            trace_id.get(),
            since.map(encode_time),
            until.map(encode_time),
            limit,
        ],
        from_row,
    )?;
    rows.collect()
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<CheckpointRecord> {
    let deletion = match optional_time(row, 5)? {
        Some(deleted_at) => Some(Deletion {
            deleted_at,
            release: release(row)?,
        }),
        None => None,
    };
    Ok(CheckpointRecord {
        checkpoint_id: CheckpointId::new(row.get(0)?),
        trace_id: TraceId::new(row.get(1)?),
        operation_id: OperationId::new(row.get(2)?),
        created_at: time(row, 3)?,
        snapshot_id: row.get(8)?,
        invalidated_by: row.get::<_, Option<u64>>(4)?.map(OperationId::new),
        deletion,
    })
}

fn release(row: &Row<'_>) -> rusqlite::Result<Release> {
    match row.get::<_, Option<String>>(6)?.as_deref() {
        Some("pending") => Ok(Release::Pending),
        Some("released") => Ok(Release::Released),
        Some("failed") => Ok(Release::Failed(json(row, 7)?)),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            6,
            Type::Text,
            format!("unknown release state {other:?}").into(),
        )),
    }
}

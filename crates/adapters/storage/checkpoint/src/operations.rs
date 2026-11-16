//! One row per save or restore: progress, result and the LLM boundary.

use std::time::SystemTime;

use checkpoint_core::model::{
    Boundary, CheckpointId, Failure, NewOperation, OperationId, OperationKind, OperationOutcome,
    OperationRecord, Phase, Trigger,
};
use model_core::ids::TraceId;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::codec::{encode_time, json, optional_json, optional_time, text_enum, time, to_json};

const SELECT_COLUMNS: &str = "SELECT operation_id, trace_id, kind, trigger_kind, phase, outcome,
    requested_at_ns, started_at_ns, finished_at_ns, target_checkpoint_id, created_checkpoint_id,
    boundary_observed_at_ns, boundary_request_action_id, failure_json,
    invalidated_checkpoints_json FROM operations";

pub(crate) fn insert(
    connection: &Connection,
    operation: &NewOperation,
) -> rusqlite::Result<OperationId> {
    connection.execute(
        "INSERT INTO operations (
            trace_id, kind, trigger_kind, phase, requested_at_ns, target_checkpoint_id,
            invalidated_checkpoints_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]')",
        params![
            operation.trace_id.get(),
            operation.kind.as_str(),
            operation.trigger.as_str(),
            operation.phase.as_str(),
            encode_time(operation.requested_at),
            operation.target_checkpoint.map(CheckpointId::get),
        ],
    )?;
    let id = u64::try_from(connection.last_insert_rowid()).expect("rowids are positive");
    Ok(OperationId::new(id))
}

pub(crate) fn start(
    connection: &Connection,
    operation_id: OperationId,
    started_at: SystemTime,
    boundary: Option<&Boundary>,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE operations SET phase = ?2, started_at_ns = ?3,
            boundary_observed_at_ns = ?4, boundary_request_action_id = ?5
         WHERE operation_id = ?1 AND phase != ?6",
        params![
            operation_id.get(),
            Phase::Executing.as_str(),
            encode_time(started_at),
            boundary.map(|boundary| encode_time(boundary.observed_at)),
            boundary.map(|boundary| boundary.request_action_id.as_str()),
            Phase::Finished.as_str(),
        ],
    )
}

pub(crate) struct Finish<'a> {
    pub(crate) finished_at: SystemTime,
    pub(crate) outcome: OperationOutcome,
    pub(crate) failure: Option<&'a Failure>,
    pub(crate) created_checkpoint: Option<CheckpointId>,
    pub(crate) invalidated: &'a [CheckpointId],
}

pub(crate) fn finish(
    connection: &Connection,
    operation_id: OperationId,
    finish: &Finish<'_>,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE operations SET phase = ?2, outcome = ?3, finished_at_ns = ?4,
            failure_json = ?5, created_checkpoint_id = ?6, invalidated_checkpoints_json = ?7
         WHERE operation_id = ?1 AND phase != ?2",
        params![
            operation_id.get(),
            Phase::Finished.as_str(),
            finish.outcome.as_str(),
            encode_time(finish.finished_at),
            finish.failure.map(to_json),
            finish.created_checkpoint.map(CheckpointId::get),
            to_json(
                &finish
                    .invalidated
                    .iter()
                    .map(|checkpoint| checkpoint.get())
                    .collect::<Vec<_>>()
            ),
        ],
    )
}

pub(crate) fn kind(
    connection: &Connection,
    operation_id: OperationId,
) -> rusqlite::Result<Option<(TraceId, OperationKind)>> {
    connection
        .query_row(
            "SELECT trace_id, kind FROM operations WHERE operation_id = ?1",
            [operation_id.get()],
            |row| {
                Ok((
                    TraceId::new(row.get(0)?),
                    text_enum(row, 1, OperationKind::parse)?,
                ))
            },
        )
        .optional()
}

pub(crate) fn get(
    connection: &Connection,
    operation_id: OperationId,
) -> rusqlite::Result<Option<OperationRecord>> {
    connection
        .query_row(
            &format!("{SELECT_COLUMNS} WHERE operation_id = ?1"),
            [operation_id.get()],
            from_row,
        )
        .optional()
}

pub(crate) fn for_trace(
    connection: &Connection,
    trace_id: TraceId,
) -> rusqlite::Result<Vec<OperationRecord>> {
    let mut statement = connection.prepare_cached(&format!(
        "{SELECT_COLUMNS} WHERE trace_id = ?1 ORDER BY operation_id"
    ))?;
    let rows = statement.query_map([trace_id.get()], from_row)?;
    rows.collect()
}

pub(crate) fn unfinished(connection: &Connection) -> rusqlite::Result<Vec<OperationRecord>> {
    let mut statement = connection.prepare_cached(&format!(
        "{SELECT_COLUMNS} WHERE phase != ?1 ORDER BY operation_id"
    ))?;
    let rows = statement.query_map([Phase::Finished.as_str()], from_row)?;
    rows.collect()
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<OperationRecord> {
    let outcome = match row.get::<_, Option<String>>(5)? {
        Some(_) => Some(text_enum(row, 5, OperationOutcome::parse)?),
        None => None,
    };
    let boundary = match optional_time(row, 11)? {
        Some(observed_at) => Some(Boundary {
            observed_at,
            request_action_id: row.get(12)?,
        }),
        None => None,
    };
    Ok(OperationRecord {
        operation_id: OperationId::new(row.get(0)?),
        trace_id: TraceId::new(row.get(1)?),
        kind: text_enum(row, 2, OperationKind::parse)?,
        trigger: text_enum(row, 3, Trigger::parse)?,
        phase: text_enum(row, 4, Phase::parse)?,
        outcome,
        requested_at: time(row, 6)?,
        started_at: optional_time(row, 7)?,
        finished_at: optional_time(row, 8)?,
        target_checkpoint: row.get::<_, Option<u64>>(9)?.map(CheckpointId::new),
        created_checkpoint: row.get::<_, Option<u64>>(10)?.map(CheckpointId::new),
        boundary,
        failure: optional_json(row, 13)?,
        invalidated_checkpoints: json::<Vec<u64>>(row, 14)?
            .into_iter()
            .map(CheckpointId::new)
            .collect(),
    })
}

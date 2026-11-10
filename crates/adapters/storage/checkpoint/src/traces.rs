//! One row per checkpoint-enabled Trace: owner, root process and scopes.

use std::time::SystemTime;

use checkpoint_core::model::{Disconnected, RootProcess, TraceOwner, TraceRecord};
use model_core::ids::TraceId;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::codec::{encode_time, json, optional_json, optional_time, time, to_json};

pub(crate) fn next_id_seed(connection: &Connection) -> rusqlite::Result<u64> {
    connection.query_row("SELECT COALESCE(MAX(trace_id), 0) FROM traces", [], |row| {
        row.get::<_, u64>(0).map(|maximum| maximum + 1)
    })
}

pub(crate) fn insert(connection: &Connection, trace: &TraceRecord) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO traces (
            trace_id, owner_uid, owner_pid_namespace, owner_mount_namespace,
            owner_host_pid_namespace, owner_host_mount_namespace, root_host_pid,
            root_start_time_ticks, scopes_json, created_at_ns, ended_at_ns, disconnected_at_ns,
            disconnect_failure_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            trace.trace_id.get(),
            trace.owner.uid,
            trace.owner.pid_namespace,
            trace.owner.mount_namespace,
            trace.owner.host_pid_namespace,
            trace.owner.host_mount_namespace,
            trace.root.host_pid,
            trace.root.start_time_ticks,
            to_json(&trace.scopes),
            encode_time(trace.created_at),
            trace.ended_at.map(encode_time),
            trace
                .disconnected
                .as_ref()
                .map(|disconnected| encode_time(disconnected.disconnected_at)),
            trace
                .disconnected
                .as_ref()
                .and_then(|disconnected| disconnected.failure.as_ref())
                .map(to_json),
        ],
    )?;
    Ok(())
}

pub(crate) fn get(
    connection: &Connection,
    trace_id: TraceId,
) -> rusqlite::Result<Option<TraceRecord>> {
    connection
        .query_row(
            "SELECT trace_id, owner_uid, owner_pid_namespace, owner_mount_namespace,
                owner_host_pid_namespace, owner_host_mount_namespace, root_host_pid,
                root_start_time_ticks, scopes_json, created_at_ns, ended_at_ns, disconnected_at_ns,
                disconnect_failure_json
             FROM traces WHERE trace_id = ?1",
            [trace_id.get()],
            from_row,
        )
        .optional()
}

/// Keeps the first end time if the Trace is reported as ended twice.
pub(crate) fn mark_ended(
    connection: &Connection,
    trace_id: TraceId,
    ended_at: SystemTime,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE traces SET ended_at_ns = COALESCE(ended_at_ns, ?2) WHERE trace_id = ?1",
        params![trace_id.get(), encode_time(ended_at)],
    )
}

pub(crate) fn record_disconnect(
    connection: &Connection,
    trace_id: TraceId,
    disconnected: &Disconnected,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE traces SET disconnected_at_ns = ?2, disconnect_failure_json = ?3 WHERE trace_id = ?1",
        params![
            trace_id.get(),
            encode_time(disconnected.disconnected_at),
            disconnected.failure.as_ref().map(to_json),
        ],
    )
}

pub(crate) fn unsettled(connection: &Connection) -> rusqlite::Result<Vec<TraceId>> {
    let mut statement = connection.prepare_cached(
        "SELECT trace_id FROM traces
         WHERE ended_at_ns IS NULL OR disconnected_at_ns IS NULL OR disconnect_failure_json IS NOT NULL
            OR trace_id IN (SELECT trace_id FROM checkpoints WHERE release_state = 'pending')",
    )?;
    let rows = statement.query_map([], |row| Ok(TraceId::new(row.get(0)?)))?;
    rows.collect()
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<TraceRecord> {
    let disconnected = match optional_time(row, 11)? {
        Some(disconnected_at) => Some(Disconnected {
            disconnected_at,
            failure: optional_json(row, 12)?,
        }),
        None => None,
    };
    Ok(TraceRecord {
        trace_id: TraceId::new(row.get(0)?),
        owner: TraceOwner {
            uid: row.get(1)?,
            pid_namespace: row.get(2)?,
            mount_namespace: row.get(3)?,
            host_pid_namespace: row.get(4)?,
            host_mount_namespace: row.get(5)?,
        },
        root: RootProcess {
            host_pid: row.get(6)?,
            start_time_ticks: row.get(7)?,
        },
        scopes: json(row, 8)?,
        created_at: time(row, 9)?,
        ended_at: optional_time(row, 10)?,
        disconnected,
    })
}

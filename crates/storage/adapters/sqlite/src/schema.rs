//! Schema boundaries for traces, events, diagnostics, and tombstones.

use rusqlite::Connection;
use std::time::Duration;

const SQLITE_SCHEMA_VERSION_NANOS_TIME: i32 = 1;

const CREATE_TABLES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS traces (
    trace_id INTEGER PRIMARY KEY,
    root_pid INTEGER NOT NULL,
    root_task_id INTEGER,
    root_start_ticks INTEGER NOT NULL,
    root_pid_namespace TEXT,
    root_generation INTEGER NOT NULL,
    display_name TEXT NOT NULL,
    profile_name TEXT NOT NULL,
    tags TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL,
    health TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    failed_at INTEGER
);

CREATE TABLE IF NOT EXISTS memberships (
    trace_id INTEGER NOT NULL,
    pid INTEGER NOT NULL,
    task_id INTEGER,
    start_ticks INTEGER NOT NULL,
    pid_namespace TEXT,
    generation INTEGER NOT NULL,
    inherited_from_pid INTEGER,
    inherited_from_task_id INTEGER,
    inherited_from_start_ticks INTEGER,
    inherited_from_pid_namespace TEXT,
    inherited_from_generation INTEGER,
    observed_at INTEGER,
    capture_enabled INTEGER NOT NULL,
    propagation_enabled INTEGER NOT NULL,
    membership_state TEXT NOT NULL,
    exit_code INTEGER,
    exit_observed_at INTEGER,
    exit_observation_source TEXT,
    PRIMARY KEY (trace_id, pid, start_ticks, generation)
);

CREATE TABLE IF NOT EXISTS events (
    event_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_pid INTEGER NOT NULL,
    process_task_id INTEGER,
    process_start_ticks INTEGER NOT NULL,
    process_pid_namespace TEXT,
    process_generation INTEGER NOT NULL,
    collector TEXT NOT NULL,
    kind TEXT NOT NULL,
    bootstrap_observed INTEGER NOT NULL,
    metadata_partial INTEGER NOT NULL,
    policy_modified INTEGER NOT NULL,
    payload_variant TEXT NOT NULL,
    payload_fields TEXT NOT NULL,
    payload_bytes TEXT NOT NULL,
    policy_verdict TEXT NOT NULL,
    policy_note TEXT,
    policy_redactions TEXT NOT NULL,
    policy_truncations TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS payload_segments (
    segment_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_pid INTEGER NOT NULL,
    process_task_id INTEGER,
    process_start_ticks INTEGER NOT NULL,
    process_pid_namespace TEXT,
    process_generation INTEGER NOT NULL,
    source_boundary TEXT NOT NULL,
    content_state TEXT NOT NULL,
    direction TEXT NOT NULL,
    stream_key TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    original_size INTEGER NOT NULL,
    captured_size INTEGER NOT NULL,
    operation_id INTEGER NOT NULL DEFAULT 0,
    operation_offset INTEGER NOT NULL DEFAULT 0,
    operation_original_size INTEGER NOT NULL DEFAULT 0,
    operation_captured_size INTEGER NOT NULL DEFAULT 0,
    operation_completion_state TEXT NOT NULL DEFAULT 'unknown',
    truncation_state TEXT NOT NULL,
    redaction_state TEXT NOT NULL,
    library TEXT NOT NULL,
    symbol TEXT NOT NULL,
    protocol_hint TEXT,
    bytes BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_actions (
    action_id TEXT PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    start_time INTEGER NOT NULL,
    end_time INTEGER,
    process_pid INTEGER NOT NULL,
    process_task_id INTEGER,
    process_start_ticks INTEGER NOT NULL,
    process_pid_namespace TEXT,
    process_generation INTEGER NOT NULL,
    status TEXT NOT NULL,
    completeness TEXT NOT NULL,
    confidence_millis INTEGER,
    attributes TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_action_evidence (
    action_id TEXT NOT NULL,
    evidence_order INTEGER NOT NULL,
    kind TEXT NOT NULL,
    evidence_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (action_id, evidence_order)
);

CREATE TABLE IF NOT EXISTS semantic_action_links (
    trace_id INTEGER NOT NULL,
    parent_action_id TEXT NOT NULL,
    child_action_id TEXT NOT NULL,
    role TEXT NOT NULL,
    confidence TEXT NOT NULL,
    attributes TEXT NOT NULL,
    PRIMARY KEY (trace_id, parent_action_id, child_action_id, role)
);

CREATE TABLE IF NOT EXISTS semantic_action_link_evidence (
    trace_id INTEGER NOT NULL,
    parent_action_id TEXT NOT NULL,
    child_action_id TEXT NOT NULL,
    role TEXT NOT NULL,
    evidence_order INTEGER NOT NULL,
    kind TEXT NOT NULL,
    evidence_id INTEGER NOT NULL,
    evidence_role TEXT NOT NULL,
    PRIMARY KEY (trace_id, parent_action_id, child_action_id, role, evidence_order)
);

CREATE TABLE IF NOT EXISTS diagnostics (
    diagnostic_id INTEGER PRIMARY KEY,
    trace_id INTEGER,
    process_pid INTEGER,
    process_task_id INTEGER,
    process_start_ticks INTEGER,
    process_pid_namespace TEXT,
    process_generation INTEGER,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    emitted_at INTEGER NOT NULL,
    message TEXT NOT NULL,
    metadata TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tombstones (
    trace_id INTEGER PRIMARY KEY,
    lifecycle_state TEXT NOT NULL,
    health TEXT NOT NULL,
    cleaned_at INTEGER NOT NULL,
    cleanup_reason TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memberships_trace_parent ON memberships (
    trace_id,
    inherited_from_pid,
    inherited_from_task_id,
    inherited_from_start_ticks,
    inherited_from_pid_namespace,
    inherited_from_generation
);

CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_process_kind ON semantic_actions (
    trace_id,
    process_pid,
    process_task_id,
    process_start_ticks,
    process_pid_namespace,
    process_generation,
    kind
);

CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_child_role ON semantic_action_links (
    trace_id,
    child_action_id,
    role
);
"#;

pub fn initialize(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(CREATE_TABLES_SQL)?;
    migrate_membership_timing_columns(connection)?;
    migrate_payload_operation_columns(connection)?;
    migrate_time_columns_to_nanos(connection)?;
    migrate_query_indexes(connection)
}

fn migrate_query_indexes(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_events_trace_id ON events(trace_id);
         CREATE INDEX IF NOT EXISTS idx_payload_segments_trace_id ON payload_segments(trace_id);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_start ON semantic_actions(trace_id, start_time);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_parent ON semantic_action_links(trace_id, parent_action_id);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_child ON semantic_action_links(trace_id, child_action_id);",
    )
}

pub fn validate_read_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    if user_version(connection)? < SQLITE_SCHEMA_VERSION_NANOS_TIME {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn migrate_membership_timing_columns(connection: &Connection) -> Result<(), rusqlite::Error> {
    add_column_if_missing(connection, "memberships", "observed_at", "INTEGER")?;
    add_column_if_missing(connection, "memberships", "exit_observation_source", "TEXT")
}

fn migrate_time_columns_to_nanos(connection: &Connection) -> Result<(), rusqlite::Error> {
    if user_version(connection)? >= SQLITE_SCHEMA_VERSION_NANOS_TIME {
        return Ok(());
    }

    let nanos_per_second = i64::try_from(Duration::from_secs(1).as_nanos())
        .expect("nanoseconds per second exceed i64");
    connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        for (table, column) in [
            ("traces", "created_at"),
            ("traces", "started_at"),
            ("traces", "completed_at"),
            ("traces", "failed_at"),
            ("memberships", "observed_at"),
            ("memberships", "exit_observed_at"),
            ("events", "observed_at"),
            ("payload_segments", "observed_at"),
            ("semantic_actions", "start_time"),
            ("semantic_actions", "end_time"),
            ("diagnostics", "emitted_at"),
            ("tombstones", "cleaned_at"),
        ] {
            connection.execute(
                &format!("UPDATE {table} SET {column} = {column} * ?1 WHERE {column} IS NOT NULL"),
                [nanos_per_second],
            )?;
        }
        connection.pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION_NANOS_TIME)?;
        Ok(())
    })();
    match result {
        Ok(()) => connection.execute_batch("COMMIT"),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn user_version(connection: &Connection) -> Result<i32, rusqlite::Error> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

fn migrate_payload_operation_columns(connection: &Connection) -> Result<(), rusqlite::Error> {
    add_column_if_missing(
        connection,
        "payload_segments",
        "operation_id",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "payload_segments",
        "operation_offset",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "payload_segments",
        "operation_original_size",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "payload_segments",
        "operation_captured_size",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        connection,
        "payload_segments",
        "operation_completion_state",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }
    connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

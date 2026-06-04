//! Schema boundaries for traces, events, diagnostics, and tombstones.

use rusqlite::Connection;

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
    capture_enabled INTEGER NOT NULL,
    propagation_enabled INTEGER NOT NULL,
    membership_state TEXT NOT NULL,
    exit_code INTEGER,
    exit_observed_at INTEGER,
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
"#;

pub fn initialize(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(CREATE_TABLES_SQL)?;
    migrate_payload_operation_columns(connection)
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

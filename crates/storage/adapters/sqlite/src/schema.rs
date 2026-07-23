//! Schema boundaries for traces, events, diagnostics, and tombstones.

use rusqlite::Connection;

use crate::semantic_actions::{codebook, storage_meta};

const SQLITE_SCHEMA_VERSION_CURRENT: i32 = storage_meta::CURRENT_SCHEMA_VERSION;
const CREATE_TABLES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS process_id_sequence (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    next_process_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS processes (
    process_id INTEGER PRIMARY KEY,
    host_pid INTEGER,
    host_task_id INTEGER,
    host_start_ticks INTEGER,
    host_start_boottime_ns INTEGER,
    resolution_state TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS process_namespace_aliases (
    process_id INTEGER NOT NULL,
    pid_namespace TEXT NOT NULL,
    namespace_pid INTEGER NOT NULL,
    namespace_start_ticks INTEGER NOT NULL,
    PRIMARY KEY (process_id, pid_namespace, namespace_pid, namespace_start_ticks),
    UNIQUE (pid_namespace, namespace_pid, namespace_start_ticks)
);

INSERT OR IGNORE INTO process_id_sequence (singleton, next_process_id) VALUES (1, 1);

CREATE TABLE IF NOT EXISTS traces (
    trace_id INTEGER PRIMARY KEY,
    alert_token BLOB NOT NULL,
    root_process_id INTEGER NOT NULL,
    root_container_id TEXT,
    root_working_directory TEXT,
    display_name TEXT NOT NULL,
    profile_name TEXT NOT NULL,
    tags TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL,
    health TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    exited_at INTEGER,
    failed_at INTEGER
);

CREATE TABLE IF NOT EXISTS memberships (
    trace_id INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    inherited_from_process_id INTEGER,
    observed_at INTEGER,
    capture_enabled INTEGER NOT NULL,
    propagation_enabled INTEGER NOT NULL,
    membership_state TEXT NOT NULL,
    exit_code INTEGER,
    exit_observed_at INTEGER,
    exit_observation_source TEXT,
    PRIMARY KEY (trace_id, process_id)
);

CREATE TABLE IF NOT EXISTS events (
    event_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
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
    process_id INTEGER NOT NULL,
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

CREATE TABLE IF NOT EXISTS semantic_action_ids (
    action_key INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    action_id TEXT NOT NULL UNIQUE,
    action_id_hash BLOB NOT NULL,
    UNIQUE (trace_id, action_id_hash, action_id)
);

CREATE TABLE IF NOT EXISTS semantic_actions (
    action_key INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_time INTEGER NOT NULL,
    end_time INTEGER,
    process_id INTEGER NOT NULL,
    status_code INTEGER NOT NULL,
    completeness_code INTEGER NOT NULL,
    confidence_millis INTEGER,
    action_valid_code INTEGER NOT NULL DEFAULT 1,
    agent_observed INTEGER NOT NULL DEFAULT 0,
    process_parent_conflict INTEGER NOT NULL DEFAULT 0,
    attributes TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_action_evidence (
    action_key INTEGER NOT NULL,
    evidence_order INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    evidence_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (action_key, evidence_order)
);

CREATE TABLE IF NOT EXISTS semantic_action_links (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    confidence_code INTEGER NOT NULL,
    valid INTEGER NOT NULL DEFAULT 1,
    link_valid_code INTEGER NOT NULL DEFAULT 1,
    attributes TEXT NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code)
);

CREATE TABLE IF NOT EXISTS semantic_action_link_evidence (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    evidence_order INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    evidence_id INTEGER NOT NULL,
    evidence_role TEXT NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code, evidence_order)
);

CREATE TABLE IF NOT EXISTS semantic_action_cold_fields (
    owner_key INTEGER NOT NULL,
    field_code INTEGER NOT NULL,
    encoding_code INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    value_hash BLOB NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (owner_key, field_code)
);

CREATE TABLE IF NOT EXISTS semantic_action_link_cold_fields (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    field_code INTEGER NOT NULL,
    encoding_code INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    value_hash BLOB NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code, field_code)
);

CREATE TABLE IF NOT EXISTS file_observation_paths (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_order INTEGER NOT NULL,
    path TEXT NOT NULL,
    PRIMARY KEY (trace_id, action_key, path)
);

CREATE TABLE IF NOT EXISTS file_paths (
    path_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    path_hash TEXT NOT NULL,
    path_text TEXT NOT NULL,
    UNIQUE (trace_id, path_hash, path_text)
);

CREATE TABLE IF NOT EXISTS file_path_sets (
    trace_id INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    path_set_hash TEXT NOT NULL,
    state TEXT NOT NULL,
    unique_path_count INTEGER NOT NULL,
    stored_path_count INTEGER NOT NULL,
    chunking_scheme TEXT NOT NULL,
    PRIMARY KEY (trace_id, path_set_id)
);

CREATE TABLE IF NOT EXISTS file_path_set_action_refs (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    PRIMARY KEY (trace_id, action_key)
);

CREATE TABLE IF NOT EXISTS file_path_set_chunks (
    trace_id INTEGER NOT NULL,
    chunk_id TEXT NOT NULL,
    chunk_hash TEXT NOT NULL,
    item_count INTEGER NOT NULL,
    encoded_sorted_path_ids TEXT NOT NULL,
    chunking_scheme TEXT NOT NULL,
    PRIMARY KEY (trace_id, chunk_id),
    UNIQUE (trace_id, chunking_scheme, chunk_hash, encoded_sorted_path_ids)
);

CREATE TABLE IF NOT EXISTS file_path_set_chunk_refs (
    trace_id INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    chunk_order INTEGER NOT NULL,
    chunk_id TEXT NOT NULL,
    PRIMARY KEY (trace_id, path_set_id, chunk_order)
);

CREATE TABLE IF NOT EXISTS llm_request_manifests (
    manifest_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    format_version INTEGER NOT NULL,
    canonical_body_hash BLOB NOT NULL,
    canonical_body_bytes INTEGER NOT NULL,
    skeleton_json TEXT NOT NULL,
    UNIQUE (trace_id, action_key)
);

CREATE TABLE IF NOT EXISTS llm_request_blocks (
    block_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    block_hash BLOB NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    encoded_bytes BLOB NOT NULL,
    UNIQUE (trace_id, block_hash)
);

CREATE TABLE IF NOT EXISTS llm_request_block_refs (
    manifest_id INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    block_id INTEGER NOT NULL,
    PRIMARY KEY (manifest_id, ordinal)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS diagnostics (
    diagnostic_id INTEGER PRIMARY KEY,
    trace_id INTEGER,
    process_id INTEGER,
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
    inherited_from_process_id
);

CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_process_kind ON semantic_actions (
    trace_id,
    process_id,
    kind_code
);

CREATE INDEX IF NOT EXISTS idx_processes_host_pid ON processes (host_pid);
CREATE INDEX IF NOT EXISTS idx_process_alias_namespace_pid
    ON process_namespace_aliases (pid_namespace, namespace_pid);

CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_child_role ON semantic_action_links (
    trace_id,
    child_action_key,
    role_code
);

CREATE INDEX IF NOT EXISTS idx_file_observation_paths_action_order ON file_observation_paths (
    trace_id,
    action_key,
    path_order
);

CREATE INDEX IF NOT EXISTS idx_file_paths_trace_text ON file_paths (
    trace_id,
    path_text
);

CREATE INDEX IF NOT EXISTS idx_file_path_set_refs_path_set ON file_path_set_chunk_refs (
    trace_id,
    path_set_id,
    chunk_order
);

CREATE INDEX IF NOT EXISTS idx_file_path_set_action_refs_path_set ON file_path_set_action_refs (
    trace_id,
    path_set_id
);

"#;

pub fn initialize(connection: &Connection) -> Result<(), rusqlite::Error> {
    let version = user_version(connection)?;
    validate_writable_schema_state(connection, version)?;
    connection.execute_batch(CREATE_TABLES_SQL)?;
    connection.execute_batch(crate::alerts::schema::CREATE_SQL)?;
    codebook::for_schema_version(SQLITE_SCHEMA_VERSION_CURRENT)
        .and_then(|codebook| codebook.validate())
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    validate_current_schema(connection)?;
    connection.pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION_CURRENT)?;
    migrate_query_indexes(connection)
}

fn migrate_query_indexes(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_events_trace_id ON events(trace_id);
         CREATE INDEX IF NOT EXISTS idx_payload_segments_trace_id ON payload_segments(trace_id);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_start ON semantic_actions(trace_id, start_time);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_parent ON semantic_action_links(trace_id, parent_action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_child ON semantic_action_links(trace_id, child_action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_valid_parent ON semantic_action_links(trace_id, valid, parent_action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_valid_child ON semantic_action_links(trace_id, valid, child_action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_valid_role ON semantic_action_links(trace_id, valid, role_code);",
    )
}

pub fn validate_read_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    if user_version(connection)? != SQLITE_SCHEMA_VERSION_CURRENT {
        return Err(rusqlite::Error::InvalidQuery);
    }
    codebook::for_schema_version(SQLITE_SCHEMA_VERSION_CURRENT)
        .and_then(|codebook| codebook.validate())
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    validate_current_schema(connection)?;
    Ok(())
}

fn validate_writable_schema_state(
    connection: &Connection,
    version: i32,
) -> Result<(), rusqlite::Error> {
    if version == SQLITE_SCHEMA_VERSION_CURRENT {
        return Ok(());
    }
    if version == 0 && user_table_count(connection)? == 0 {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn validate_current_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    crate::alerts::schema::validate(connection)?;
    require_column(connection, "processes", "process_id")?;
    require_column(connection, "process_namespace_aliases", "process_id")?;
    require_column(connection, "traces", "alert_token")?;
    require_column(connection, "traces", "root_process_id")?;
    require_column(connection, "traces", "root_working_directory")?;
    require_column(connection, "memberships", "process_id")?;
    require_column(connection, "events", "process_id")?;
    require_column(connection, "payload_segments", "process_id")?;
    require_column(connection, "traces", "exited_at")?;
    require_column(connection, "semantic_action_ids", "action_key")?;
    require_column(connection, "semantic_action_ids", "action_id")?;
    require_column(connection, "semantic_action_ids", "action_id_hash")?;
    require_column(connection, "semantic_actions", "action_key")?;
    require_column(connection, "semantic_actions", "kind_code")?;
    require_column(connection, "semantic_actions", "status_code")?;
    require_column(connection, "semantic_actions", "completeness_code")?;
    require_column(connection, "semantic_actions", "action_valid_code")?;
    require_column(connection, "semantic_actions", "agent_observed")?;
    require_column(connection, "semantic_actions", "process_parent_conflict")?;
    require_column(connection, "semantic_action_evidence", "action_key")?;
    require_column(connection, "semantic_action_evidence", "kind_code")?;
    require_column(connection, "semantic_action_links", "parent_action_key")?;
    require_column(connection, "semantic_action_links", "child_action_key")?;
    require_column(connection, "semantic_action_links", "role_code")?;
    require_column(connection, "semantic_action_links", "confidence_code")?;
    require_column(connection, "semantic_action_links", "link_valid_code")?;
    require_column(
        connection,
        "semantic_action_link_evidence",
        "parent_action_key",
    )?;
    require_column(
        connection,
        "semantic_action_link_evidence",
        "child_action_key",
    )?;
    require_column(connection, "semantic_action_link_evidence", "role_code")?;
    require_column(connection, "semantic_action_link_evidence", "kind_code")?;
    require_column(connection, "file_observation_paths", "action_key")?;
    require_column(connection, "file_path_sets", "path_set_hash")?;
    require_column(connection, "file_path_set_action_refs", "action_key")?;
    require_column(connection, "llm_request_manifests", "manifest_id")?;
    require_column(connection, "llm_request_manifests", "action_key")?;
    require_column(connection, "llm_request_blocks", "block_id")?;
    require_column(connection, "llm_request_block_refs", "manifest_id")?;
    require_column(connection, "semantic_action_cold_fields", "payload")?;
    require_column(connection, "semantic_action_link_cold_fields", "payload")
}

fn user_version(connection: &Connection) -> Result<i32, rusqlite::Error> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

fn user_table_count(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )
}

fn require_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<(), rusqlite::Error> {
    if column_exists(connection, table, column)? {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

//! Tables of the checkpoint catalog and its schema version.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

pub(crate) const SCHEMA_VERSION: i64 = 4;

const CREATE_SCHEMA_SQL: &str = "
CREATE TABLE checkpoint_schema (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    version INTEGER NOT NULL
);
CREATE TABLE traces (
    trace_id INTEGER PRIMARY KEY,
    owner_uid INTEGER NOT NULL,
    owner_pid_namespace TEXT NOT NULL,
    owner_mount_namespace TEXT NOT NULL,
    owner_host_pid_namespace INTEGER NOT NULL,
    owner_host_mount_namespace INTEGER NOT NULL,
    root_host_pid INTEGER NOT NULL,
    root_start_time_ticks INTEGER NOT NULL,
    scopes_json TEXT NOT NULL,
    created_at_ns INTEGER NOT NULL,
    ended_at_ns INTEGER,
    disconnected_at_ns INTEGER,
    disconnect_failure_json TEXT
);
CREATE TABLE operations (
    operation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id INTEGER NOT NULL REFERENCES traces (trace_id),
    kind TEXT NOT NULL,
    trigger_kind TEXT NOT NULL,
    phase TEXT NOT NULL,
    outcome TEXT,
    requested_at_ns INTEGER NOT NULL,
    started_at_ns INTEGER,
    finished_at_ns INTEGER,
    target_checkpoint_id INTEGER,
    created_checkpoint_id INTEGER,
    boundary_observed_at_ns INTEGER,
    boundary_request_action_id TEXT,
    failure_json TEXT,
    invalidated_checkpoints_json TEXT NOT NULL,
    FOREIGN KEY (trace_id, target_checkpoint_id) REFERENCES checkpoints (trace_id, checkpoint_id),
    FOREIGN KEY (trace_id, created_checkpoint_id) REFERENCES checkpoints (trace_id, checkpoint_id)
);
CREATE INDEX operations_trace_idx ON operations (trace_id, operation_id);
CREATE TABLE checkpoints (
    trace_id INTEGER NOT NULL REFERENCES traces (trace_id),
    -- Numbered from 0 within each Trace, 0 being the initial checkpoint; deleted checkpoints keep
    -- their row and number.
    checkpoint_id INTEGER NOT NULL,
    operation_id INTEGER NOT NULL REFERENCES operations (operation_id),
    created_at_ns INTEGER NOT NULL,
    snapshot_id BLOB NOT NULL,
    invalidated_by INTEGER REFERENCES operations (operation_id),
    deleted_at_ns INTEGER,
    release_state TEXT CHECK (release_state IN ('pending', 'released', 'failed')),
    release_failure_json TEXT,
    PRIMARY KEY (trace_id, checkpoint_id)
);
";

/// Creates the tables in an empty database, or checks the version of an existing catalog.
pub(crate) fn initialize(connection: &mut Connection) -> Result<(), String> {
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("enable WAL: {error}"))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| format!("enable foreign keys: {error}"))?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let initialized = transaction
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_schema'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if initialized {
        let version: i64 = transaction
            .query_row(
                "SELECT version FROM checkpoint_schema WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("read catalog schema version: {error}"))?;
        if version != SCHEMA_VERSION {
            return Err(format!(
                "catalog schema version {version} is not the supported version {SCHEMA_VERSION}"
            ));
        }
        return Ok(());
    }
    let tables: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if tables != 0 {
        return Err("database is not a checkpoint catalog".to_string());
    }
    transaction
        .execute_batch(CREATE_SCHEMA_SQL)
        .map_err(|error| format!("create schema: {error}"))?;
    transaction
        .execute(
            "INSERT INTO checkpoint_schema (singleton, version) VALUES (1, ?1)",
            [SCHEMA_VERSION],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

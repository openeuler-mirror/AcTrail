//! Semantic action schema migration from public strings to storage codes.

use rusqlite::{Connection, params};
use semantic_action::attr_keys as attrs;
use sha2::{Digest, Sha256};

use crate::records::decode_map;
use crate::semantic_actions::codebook;

use super::semantic_references;

const CREATE_SEMANTIC_ACTION_CODEBOOK_TABLES_SQL: &str = r#"
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
    process_pid INTEGER NOT NULL,
    process_task_id INTEGER,
    process_start_ticks INTEGER NOT NULL,
    process_pid_namespace TEXT,
    process_generation INTEGER NOT NULL,
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
"#;

pub(crate) fn migrate_semantic_action_codebook(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if !table_exists(connection, "semantic_actions")?
        || column_exists(connection, "semantic_actions", "kind_code")?
    {
        return Ok(());
    }
    codebook::current()
        .validate()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;

    connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = migrate_semantic_action_codebook_inner(connection);
    match result {
        Ok(()) => connection.execute_batch("COMMIT"),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn migrate_semantic_action_codebook_inner(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "DROP INDEX IF EXISTS idx_semantic_actions_trace_process_kind;
         DROP INDEX IF EXISTS idx_semantic_actions_trace_start;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_child_role;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_parent;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_child;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_valid_parent;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_valid_child;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_valid_role;
         DROP INDEX IF EXISTS idx_file_observation_paths_action_order;
         DROP INDEX IF EXISTS idx_file_path_set_action_refs_path_set;
         DROP TABLE IF EXISTS file_observation_paths_v6;
         DROP TABLE IF EXISTS file_path_set_action_refs_v6;
         DROP TABLE IF EXISTS llm_request_manifests_v6;
         DROP TABLE IF EXISTS semantic_action_link_evidence_v6;
         DROP TABLE IF EXISTS semantic_action_links_v6;
         DROP TABLE IF EXISTS semantic_action_evidence_v6;
         DROP TABLE IF EXISTS semantic_actions_v6;
         ALTER TABLE semantic_actions RENAME TO semantic_actions_v6;
         ALTER TABLE semantic_action_evidence RENAME TO semantic_action_evidence_v6;
         ALTER TABLE semantic_action_links RENAME TO semantic_action_links_v6;
         ALTER TABLE semantic_action_link_evidence RENAME TO semantic_action_link_evidence_v6;",
    )?;
    semantic_references::rename_legacy_tables(connection)?;
    connection.execute_batch(CREATE_SEMANTIC_ACTION_CODEBOOK_TABLES_SQL)?;
    connection.execute_batch(semantic_references::CREATE_REFERENCE_TABLES_SQL)?;
    migrate_semantic_action_rows(connection)?;
    migrate_semantic_action_evidence_rows(connection)?;
    migrate_semantic_action_link_rows(connection)?;
    migrate_semantic_action_link_evidence_rows(connection)?;
    semantic_references::migrate_rows(connection)?;
    connection.execute_batch(
        "DROP TABLE IF EXISTS llm_request_manifests_v6;
         DROP TABLE IF EXISTS file_path_set_action_refs_v6;
         DROP TABLE IF EXISTS file_observation_paths_v6;
         DROP TABLE semantic_action_link_evidence_v6;
         DROP TABLE semantic_action_links_v6;
         DROP TABLE semantic_action_evidence_v6;
         DROP TABLE semantic_actions_v6;",
    )
}

fn migrate_semantic_action_rows(connection: &Connection) -> Result<(), rusqlite::Error> {
    let codes = codebook::current();
    let mut rows = connection.prepare(
        "SELECT action_id, trace_id, kind, title, start_time, end_time, process_pid,
                process_task_id, process_start_ticks, process_pid_namespace,
                process_generation, status, completeness, confidence_millis, attributes
         FROM semantic_actions_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let action_id = row.get::<_, String>("action_id")?;
        let trace_id = row.get::<_, i64>("trace_id")?;
        let action_key = intern_migrated_action_id(connection, trace_id, &action_id)?;
        connection.execute(
            "INSERT INTO semantic_actions (
                action_key, trace_id, kind_code, title, start_time, end_time, process_pid,
                process_task_id, process_start_ticks, process_pid_namespace,
                process_generation, status_code, completeness_code, confidence_millis,
                action_valid_code, agent_observed, process_parent_conflict, attributes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                action_key,
                trace_id,
                sqlite_code(
                    codes
                        .action_kind
                        .code_from_str(&row.get::<_, String>("kind")?)
                )?,
                row.get::<_, String>("title")?,
                row.get::<_, i64>("start_time")?,
                row.get::<_, Option<i64>>("end_time")?,
                row.get::<_, i64>("process_pid")?,
                row.get::<_, Option<i64>>("process_task_id")?,
                row.get::<_, i64>("process_start_ticks")?,
                row.get::<_, Option<String>>("process_pid_namespace")?,
                row.get::<_, i64>("process_generation")?,
                sqlite_code(
                    codes
                        .action_status
                        .code_from_str(&row.get::<_, String>("status")?)
                )?,
                sqlite_code(
                    codes
                        .action_completeness
                        .code_from_str(&row.get::<_, String>("completeness")?)
                )?,
                row.get::<_, Option<i64>>("confidence_millis")?,
                action_valid_code(&row.get::<_, String>("attributes")?),
                agent_observed(&row.get::<_, String>("attributes")?),
                process_parent_conflict(&row.get::<_, String>("attributes")?),
                row.get::<_, String>("attributes")?,
            ],
        )?;
    }
    Ok(())
}

fn migrate_semantic_action_evidence_rows(connection: &Connection) -> Result<(), rusqlite::Error> {
    let codes = codebook::current();
    let mut rows = connection.prepare(
        "SELECT action_id, evidence_order, kind, evidence_id, role
         FROM semantic_action_evidence_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let action_key =
            require_migrated_action_key(connection, &row.get::<_, String>("action_id")?)?;
        connection.execute(
            "INSERT INTO semantic_action_evidence (
                action_key, evidence_order, kind_code, evidence_id, role
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                action_key,
                row.get::<_, i64>("evidence_order")?,
                sqlite_code(
                    codes
                        .evidence_kind
                        .code_from_str(&row.get::<_, String>("kind")?)
                )?,
                row.get::<_, i64>("evidence_id")?,
                row.get::<_, String>("role")?,
            ],
        )?;
    }
    Ok(())
}

fn migrate_semantic_action_link_rows(connection: &Connection) -> Result<(), rusqlite::Error> {
    let codes = codebook::current();
    let mut rows = connection.prepare(
        "SELECT trace_id, parent_action_id, child_action_id, role, confidence, valid, attributes
         FROM semantic_action_links_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let parent_action_key =
            require_migrated_action_key(connection, &row.get::<_, String>("parent_action_id")?)?;
        let child_action_key =
            require_migrated_action_key(connection, &row.get::<_, String>("child_action_id")?)?;
        let valid = row.get::<_, i64>("valid")?;
        let attributes = row.get::<_, String>("attributes")?;
        connection.execute(
            "INSERT INTO semantic_action_links (
                trace_id, parent_action_key, child_action_key, role_code,
                confidence_code, valid, link_valid_code, attributes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.get::<_, i64>("trace_id")?,
                parent_action_key,
                child_action_key,
                sqlite_code(
                    codes
                        .link_role
                        .code_from_str(&row.get::<_, String>("role")?)
                )?,
                sqlite_code(
                    codes
                        .link_confidence
                        .code_from_str(&row.get::<_, String>("confidence")?)
                )?,
                valid,
                link_valid_code(valid, &attributes),
                attributes,
            ],
        )?;
    }
    Ok(())
}

fn migrate_semantic_action_link_evidence_rows(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    let codes = codebook::current();
    let mut rows = connection.prepare(
        "SELECT trace_id, parent_action_id, child_action_id, role, evidence_order,
                kind, evidence_id, evidence_role
         FROM semantic_action_link_evidence_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let parent_action_key =
            require_migrated_action_key(connection, &row.get::<_, String>("parent_action_id")?)?;
        let child_action_key =
            require_migrated_action_key(connection, &row.get::<_, String>("child_action_id")?)?;
        connection.execute(
            "INSERT INTO semantic_action_link_evidence (
                trace_id, parent_action_key, child_action_key, role_code, evidence_order,
                kind_code, evidence_id, evidence_role
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.get::<_, i64>("trace_id")?,
                parent_action_key,
                child_action_key,
                sqlite_code(
                    codes
                        .link_role
                        .code_from_str(&row.get::<_, String>("role")?)
                )?,
                row.get::<_, i64>("evidence_order")?,
                sqlite_code(
                    codes
                        .evidence_kind
                        .code_from_str(&row.get::<_, String>("kind")?)
                )?,
                row.get::<_, i64>("evidence_id")?,
                row.get::<_, String>("evidence_role")?,
            ],
        )?;
    }
    Ok(())
}

fn intern_migrated_action_id(
    connection: &Connection,
    trace_id: i64,
    action_id: &str,
) -> Result<i64, rusqlite::Error> {
    let action_id_hash = sha256_hash_blob(action_id.as_bytes());
    connection.execute(
        "INSERT OR IGNORE INTO semantic_action_ids (trace_id, action_id, action_id_hash)
         VALUES (?1, ?2, ?3)",
        params![trace_id, action_id, &action_id_hash],
    )?;
    require_migrated_action_key(connection, action_id)
}

fn require_migrated_action_key(
    connection: &Connection,
    action_id: &str,
) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "SELECT action_key FROM semantic_action_ids WHERE action_id = ?1",
        params![action_id],
        |row| row.get::<_, i64>("action_key"),
    )
}

fn sha256_hash_blob(input: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(input);
    hasher.finalize().to_vec()
}

fn action_valid_code(raw_attributes: &str) -> i16 {
    if decode_map(raw_attributes)
        .get(attrs::actrail::ACTION_VALID)
        .is_some_and(|value| value == "false")
    {
        0
    } else {
        1
    }
}

fn agent_observed(raw_attributes: &str) -> i16 {
    if decode_map(raw_attributes)
        .get(attrs::agent::IDENTITY_STATUS)
        .is_some_and(|value| value == "observed")
    {
        1
    } else {
        0
    }
}

fn process_parent_conflict(raw_attributes: &str) -> i16 {
    if decode_map(raw_attributes)
        .get(attrs::process_parent::IDENTITY_STATE)
        .is_some_and(|value| value == "conflict")
    {
        1
    } else {
        0
    }
}

fn link_valid_code(valid: i64, raw_attributes: &str) -> i16 {
    if valid == 1
        && !decode_map(raw_attributes)
            .get(attrs::actrail::LINK_VALID)
            .is_some_and(|value| value == "false")
    {
        1
    } else {
        0
    }
}

fn sqlite_code<T>(result: Result<T, codebook::CodebookError>) -> Result<T, rusqlite::Error> {
    result.map_err(|_| rusqlite::Error::InvalidQuery)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    let exists = connection.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = ?1
         )",
        params![table],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(exists)
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

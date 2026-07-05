//! Reference-table migration for semantic action id interning.

use rusqlite::{Connection, params};

pub(crate) const CREATE_REFERENCE_TABLES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS file_observation_paths (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_order INTEGER NOT NULL,
    path TEXT NOT NULL,
    PRIMARY KEY (trace_id, action_key, path)
);

CREATE TABLE IF NOT EXISTS file_path_set_action_refs (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    PRIMARY KEY (trace_id, action_key)
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
"#;

pub(crate) fn rename_legacy_tables(connection: &Connection) -> Result<(), rusqlite::Error> {
    rename_legacy_table(
        connection,
        "file_observation_paths",
        "file_observation_paths_v6",
    )?;
    rename_legacy_table(
        connection,
        "file_path_set_action_refs",
        "file_path_set_action_refs_v6",
    )?;
    rename_legacy_table(
        connection,
        "llm_request_manifests",
        "llm_request_manifests_v6",
    )
}

pub(crate) fn migrate_rows(connection: &Connection) -> Result<(), rusqlite::Error> {
    migrate_file_observation_paths(connection)?;
    migrate_file_path_set_action_refs(connection)?;
    migrate_llm_request_manifests(connection)
}

fn migrate_file_observation_paths(connection: &Connection) -> Result<(), rusqlite::Error> {
    if !table_exists(connection, "file_observation_paths_v6")? {
        return Ok(());
    }
    let mut rows = connection.prepare(
        "SELECT trace_id, action_id, path_order, path
         FROM file_observation_paths_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let trace_id = row.get::<_, i64>("trace_id")?;
        let action_key =
            require_action_key(connection, trace_id, &row.get::<_, String>("action_id")?)?;
        connection.execute(
            "INSERT INTO file_observation_paths (
                trace_id, action_key, path_order, path
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                trace_id,
                action_key,
                row.get::<_, i64>("path_order")?,
                row.get::<_, String>("path")?,
            ],
        )?;
    }
    Ok(())
}

fn migrate_file_path_set_action_refs(connection: &Connection) -> Result<(), rusqlite::Error> {
    if !table_exists(connection, "file_path_set_action_refs_v6")? {
        return Ok(());
    }
    let mut rows = connection.prepare(
        "SELECT trace_id, action_id, path_set_id
         FROM file_path_set_action_refs_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let trace_id = row.get::<_, i64>("trace_id")?;
        let action_key =
            require_action_key(connection, trace_id, &row.get::<_, String>("action_id")?)?;
        connection.execute(
            "INSERT INTO file_path_set_action_refs (
                trace_id, action_key, path_set_id
             ) VALUES (?1, ?2, ?3)",
            params![trace_id, action_key, row.get::<_, String>("path_set_id")?],
        )?;
    }
    Ok(())
}

fn migrate_llm_request_manifests(connection: &Connection) -> Result<(), rusqlite::Error> {
    if !table_exists(connection, "llm_request_manifests_v6")? {
        return Ok(());
    }
    let mut rows = connection.prepare(
        "SELECT manifest_id, trace_id, action_id, format_version, canonical_body_hash,
                canonical_body_bytes, skeleton_json
         FROM llm_request_manifests_v6",
    )?;
    let mut rows = rows.query([])?;
    while let Some(row) = rows.next()? {
        let trace_id = row.get::<_, i64>("trace_id")?;
        let action_key =
            require_action_key(connection, trace_id, &row.get::<_, String>("action_id")?)?;
        connection.execute(
            "INSERT INTO llm_request_manifests (
                manifest_id, trace_id, action_key, format_version, canonical_body_hash,
                canonical_body_bytes, skeleton_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.get::<_, i64>("manifest_id")?,
                trace_id,
                action_key,
                row.get::<_, i64>("format_version")?,
                row.get::<_, Vec<u8>>("canonical_body_hash")?,
                row.get::<_, i64>("canonical_body_bytes")?,
                row.get::<_, String>("skeleton_json")?,
            ],
        )?;
    }
    Ok(())
}

fn require_action_key(
    connection: &Connection,
    trace_id: i64,
    action_id: &str,
) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "SELECT action_key FROM semantic_action_ids
         WHERE trace_id = ?1 AND action_id = ?2",
        params![trace_id, action_id],
        |row| row.get::<_, i64>("action_key"),
    )
}

fn rename_legacy_table(
    connection: &Connection,
    table: &str,
    legacy_table: &str,
) -> Result<(), rusqlite::Error> {
    if table_exists(connection, table)?
        && column_exists(connection, table, "action_id")?
        && !column_exists(connection, table, "action_key")?
    {
        connection.execute_batch(&format!("ALTER TABLE {table} RENAME TO {legacy_table};"))?;
    }
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = ?1
         )",
        params![table],
        |row| row.get::<_, bool>(0),
    )
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

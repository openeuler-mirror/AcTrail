use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::config::{CURRENT_SCHEMA_VERSION, SandboxEvidenceSynchronous};

const CREATE_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sandbox_schema_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    last_ingest_epoch BLOB NOT NULL CHECK (length(last_ingest_epoch) = 8)
);
CREATE TABLE IF NOT EXISTS sandbox_evidence (
    record_id INTEGER PRIMARY KEY AUTOINCREMENT,
    ingest_epoch BLOB NOT NULL CHECK (length(ingest_epoch) = 8),
    gateway_id INTEGER NOT NULL,
    sb_id INTEGER NOT NULL,
    batch_sequence BLOB NOT NULL CHECK (length(batch_sequence) = 8),
    route_generation BLOB NOT NULL CHECK (length(route_generation) = 8),
    observation_index INTEGER NOT NULL,
    observation_kind INTEGER NOT NULL,
    persisted_at_ms BLOB NOT NULL CHECK (length(persisted_at_ms) = 8),
    payload BLOB NOT NULL,
    UNIQUE (ingest_epoch, gateway_id, sb_id, batch_sequence, observation_index)
);
CREATE INDEX IF NOT EXISTS sandbox_evidence_source_idx
ON sandbox_evidence (gateway_id, sb_id, record_id);
";

pub(super) struct SandboxSchema;

impl SandboxSchema {
    pub(super) fn initialize(
        connection: &mut Connection,
        expected_version: u32,
        synchronous: SandboxEvidenceSynchronous,
        wal_autocheckpoint_pages: u32,
        capacity_max_bytes: u64,
    ) -> rusqlite::Result<u64> {
        let application_tables = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get::<_, u64>(0),
        )?;
        let has_schema_meta = connection
            .query_row(
                "SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'sandbox_schema_meta'",
                [],
                |row| row.get::<_, u8>(0),
            )
            .optional()?
            .is_some();
        if application_tables != 0 && !has_schema_meta {
            return Err(rusqlite::Error::InvalidParameterName(
                "database is not an initialized sandbox evidence store".to_string(),
            ));
        }
        let ingest_epoch = if application_tables == 0 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(CREATE_SCHEMA_SQL)?;
            let ingest_epoch = 1_u64;
            transaction.execute(
                "INSERT INTO sandbox_schema_meta
                 (singleton, schema_version, last_ingest_epoch) VALUES (1, ?1, ?2)",
                rusqlite::params![expected_version, ingest_epoch.to_be_bytes().as_slice()],
            )?;
            transaction.commit()?;
            ingest_epoch
        } else {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let (version, previous_epoch) = transaction.query_row(
                "SELECT schema_version, last_ingest_epoch
                 FROM sandbox_schema_meta WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )?;
            if version != expected_version {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "sandbox evidence schema version {version} does not match configured {expected_version}"
                )));
            }
            let has_evidence_table = transaction
                .query_row(
                    "SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'sandbox_evidence'",
                    [],
                    |row| row.get::<_, u8>(0),
                )
                .optional()?
                .is_some();
            if !has_evidence_table {
                return Err(rusqlite::Error::InvalidParameterName(
                    "sandbox evidence schema is incomplete".to_string(),
                ));
            }
            let previous_epoch = decode_u64(&previous_epoch)?;
            let ingest_epoch = previous_epoch.checked_add(1).ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(
                    "sandbox evidence ingest epoch is exhausted".to_string(),
                )
            })?;
            transaction.execute(
                "UPDATE sandbox_schema_meta SET last_ingest_epoch = ?1 WHERE singleton = 1",
                [ingest_epoch.to_be_bytes().as_slice()],
            )?;
            transaction.commit()?;
            ingest_epoch
        };
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", synchronous.pragma_value())?;
        connection.pragma_update(None, "wal_autocheckpoint", wal_autocheckpoint_pages)?;
        configure_max_page_count(connection, capacity_max_bytes)?;
        Ok(ingest_epoch)
    }

    pub(super) fn verify_read_only(
        connection: &Connection,
        expected_version: u32,
    ) -> rusqlite::Result<()> {
        let version = connection.query_row(
            "SELECT schema_version FROM sandbox_schema_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, u32>(0),
        )?;
        if version != expected_version || version != CURRENT_SCHEMA_VERSION {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "sandbox evidence schema version {version} is unsupported"
            )));
        }
        Ok(())
    }
}

fn decode_u64(bytes: &[u8]) -> rusqlite::Result<u64> {
    let bytes: [u8; 8] = bytes.try_into().map_err(|_| {
        rusqlite::Error::InvalidParameterName(
            "sandbox evidence ingest epoch has invalid width".to_string(),
        )
    })?;
    Ok(u64::from_be_bytes(bytes))
}

fn configure_max_page_count(
    connection: &Connection,
    capacity_max_bytes: u64,
) -> rusqlite::Result<()> {
    let page_size = connection.query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))?;
    let page_count = connection.query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))?;
    if page_size == 0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "sandbox evidence SQLite page size is zero".to_string(),
        ));
    }
    let max_page_count = (capacity_max_bytes / page_size).min(4_294_967_294);
    if max_page_count < page_count {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "sandbox evidence database uses {} bytes, exceeding configured capacity {capacity_max_bytes}",
            page_count.saturating_mul(page_size)
        )));
    }
    connection.pragma_update(None, "max_page_count", max_page_count)
}

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::config::{CURRENT_SCHEMA_VERSION, SandboxAlertSynchronous};

const CREATE_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sandbox_alert_schema_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    last_ingest_epoch BLOB NOT NULL CHECK (length(last_ingest_epoch) = 8)
);
CREATE TABLE IF NOT EXISTS sandbox_alerts (
    alert_id INTEGER PRIMARY KEY AUTOINCREMENT,
    ingest_epoch BLOB NOT NULL CHECK (length(ingest_epoch) = 8),
    gateway_id INTEGER NOT NULL,
    sb_id INTEGER NOT NULL,
    batch_sequence BLOB NOT NULL CHECK (length(batch_sequence) = 8),
    observation_index INTEGER NOT NULL,
    alert_kind INTEGER NOT NULL,
    detected_at_ms BLOB NOT NULL CHECK (length(detected_at_ms) = 8),
    persisted_at_ms BLOB NOT NULL CHECK (length(persisted_at_ms) = 8),
    payload BLOB NOT NULL,
    UNIQUE (
        ingest_epoch, gateway_id, sb_id, batch_sequence,
        observation_index, alert_kind
    )
);
CREATE INDEX IF NOT EXISTS sandbox_alerts_source_idx
ON sandbox_alerts (gateway_id, sb_id, alert_id);
CREATE INDEX IF NOT EXISTS sandbox_alerts_kind_idx
ON sandbox_alerts (alert_kind, alert_id);
";

pub(super) struct SandboxAlertSchema;

impl SandboxAlertSchema {
    pub(super) fn initialize(
        connection: &mut Connection,
        expected_version: u32,
        synchronous: SandboxAlertSynchronous,
        wal_autocheckpoint_pages: u32,
        capacity_max_bytes: u64,
    ) -> rusqlite::Result<u64> {
        let application_tables = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get::<_, u64>(0),
        )?;
        let has_meta = connection
            .query_row(
                "SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'sandbox_alert_schema_meta'",
                [],
                |row| row.get::<_, u8>(0),
            )
            .optional()?
            .is_some();
        if application_tables != 0 && !has_meta {
            return Err(rusqlite::Error::InvalidParameterName(
                "database is not an initialized sandbox alert store".to_string(),
            ));
        }
        let ingest_epoch = if application_tables == 0 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(CREATE_SCHEMA_SQL)?;
            let ingest_epoch = 1_u64;
            transaction.execute(
                "INSERT INTO sandbox_alert_schema_meta
                 (singleton, schema_version, last_ingest_epoch) VALUES (1, ?1, ?2)",
                rusqlite::params![expected_version, ingest_epoch.to_be_bytes().as_slice()],
            )?;
            transaction.commit()?;
            ingest_epoch
        } else {
            Self::advance_ingest_epoch(connection, expected_version)?
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
            "SELECT schema_version FROM sandbox_alert_schema_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, u32>(0),
        )?;
        if version != expected_version || version != CURRENT_SCHEMA_VERSION {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "sandbox alert schema version {version} is unsupported"
            )));
        }
        let has_alerts = connection
            .query_row(
                "SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'sandbox_alerts'",
                [],
                |row| row.get::<_, u8>(0),
            )
            .optional()?
            .is_some();
        if !has_alerts {
            return Err(rusqlite::Error::InvalidParameterName(
                "sandbox alert schema is incomplete".to_string(),
            ));
        }
        Ok(())
    }

    fn advance_ingest_epoch(
        connection: &mut Connection,
        expected_version: u32,
    ) -> rusqlite::Result<u64> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (version, previous_epoch) = transaction.query_row(
            "SELECT schema_version, last_ingest_epoch
             FROM sandbox_alert_schema_meta WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        if version != expected_version {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "sandbox alert schema version {version} does not match configured {expected_version}"
            )));
        }
        let has_alerts = transaction
            .query_row(
                "SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'sandbox_alerts'",
                [],
                |row| row.get::<_, u8>(0),
            )
            .optional()?
            .is_some();
        if !has_alerts {
            return Err(rusqlite::Error::InvalidParameterName(
                "sandbox alert schema is incomplete".to_string(),
            ));
        }
        let ingest_epoch = decode_u64(&previous_epoch)?.checked_add(1).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(
                "sandbox alert ingest epoch is exhausted".to_string(),
            )
        })?;
        transaction.execute(
            "UPDATE sandbox_alert_schema_meta SET last_ingest_epoch = ?1 WHERE singleton = 1",
            [ingest_epoch.to_be_bytes().as_slice()],
        )?;
        transaction.commit()?;
        Ok(ingest_epoch)
    }
}

fn decode_u64(bytes: &[u8]) -> rusqlite::Result<u64> {
    let bytes: [u8; 8] = bytes.try_into().map_err(|_| {
        rusqlite::Error::InvalidParameterName(
            "sandbox alert ingest epoch has invalid width".to_string(),
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
            "sandbox alert SQLite page size is zero".to_string(),
        ));
    }
    let max_page_count = (capacity_max_bytes / page_size).min(4_294_967_294);
    if max_page_count < page_count {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "sandbox alert database uses {} bytes, exceeding configured capacity {capacity_max_bytes}",
            page_count.saturating_mul(page_size)
        )));
    }
    connection.pragma_update(None, "max_page_count", max_page_count)
}

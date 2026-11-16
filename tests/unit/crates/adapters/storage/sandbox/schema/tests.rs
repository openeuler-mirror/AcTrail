// Tests for the corresponding production module.
use super::*;
use crate::config::SandboxEvidenceSynchronous;

fn initialize(connection: &mut Connection, version: u32) -> rusqlite::Result<u64> {
    SandboxSchema::initialize(
        connection,
        version,
        SandboxEvidenceSynchronous::Normal,
        1000,
        1024 * 1024,
    )
}

#[test]
fn version_2_to_3_migration_is_version_only_and_keeps_table_shape() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize(&mut connection, 2).unwrap();
    initialize(&mut connection, 3).unwrap();

    let version: u32 = connection
        .query_row(
            "SELECT schema_version FROM sandbox_schema_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 3);

    let has_evidence_table: u8 = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sandbox_evidence'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_evidence_table, 1);
}

#[test]
fn unsupported_migration_is_rejected() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize(&mut connection, 2).unwrap();
    assert!(initialize(&mut connection, 4).is_err());
    assert!(initialize(&mut connection, 1).is_err());
}

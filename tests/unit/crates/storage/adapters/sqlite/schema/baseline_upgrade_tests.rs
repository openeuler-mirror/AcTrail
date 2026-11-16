// Tests for the corresponding production module.
use super::*;

fn create_dev_baseline_without_external_bindings(connection: &Connection, with_host_scopes: bool) {
    connection.execute_batch(CREATE_TABLES_SQL).unwrap();
    connection
        .execute_batch(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/semantic_actions/store/schema.sql"
        )))
        .unwrap();
    connection
        .execute_batch(crate::alerts::schema::CREATE_SQL)
        .unwrap();
    if !with_host_scopes {
        connection
            .execute_batch("DROP TABLE trace_resource_scopes")
            .unwrap();
    }
    connection
        .pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION_CURRENT)
        .unwrap();
}

#[test]
fn archived_database_opens_read_only_without_creating_resource_registry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("archive.sqlite");
    let connection = Connection::open(&path).unwrap();
    create_dev_baseline_without_external_bindings(&connection, false);
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    let storage = crate::SqliteStorage::open_read_only(&path).unwrap();
    assert!(
        storage_core::StorageBackend::list_events(&storage, model_core::ids::TraceId::new(1))
            .unwrap()
            .is_empty()
    );
    drop(storage);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let connection =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        user_version(&connection).unwrap(),
        SQLITE_SCHEMA_VERSION_CURRENT
    );
    assert!(!column_exists(&connection, "trace_resource_scopes", "trace_id").unwrap());
    assert!(!column_exists(&connection, "trace_external_cgroup_v2_bindings", "trace_id").unwrap());
}

#[test]
fn dev_and_host_databases_gain_external_bindings() {
    for with_host_scopes in [false, true] {
        let connection = Connection::open_in_memory().unwrap();
        create_dev_baseline_without_external_bindings(&connection, with_host_scopes);
        initialize(&connection).unwrap();
        assert_eq!(
            user_version(&connection).unwrap(),
            SQLITE_SCHEMA_VERSION_CURRENT
        );
        require_schema_object(&connection, "table", "trace_resource_scopes").unwrap();
        require_schema_object(&connection, "table", "trace_external_cgroup_v2_bindings").unwrap();
        validate_read_schema(&connection).unwrap();
        initialize(&connection).unwrap();
    }
}

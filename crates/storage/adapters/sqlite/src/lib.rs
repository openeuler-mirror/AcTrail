//! SQLite-backed storage adapter.

pub mod backend;
pub mod config;
pub mod query;
pub mod records;
pub mod retention;
pub mod schema;
mod schema_migrations;
pub mod semantic_actions;
pub mod transaction;
pub mod writer;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use model_core::ids::TraceId;
use rusqlite::{Connection, OpenFlags};

pub use config::{
    SQLITE_DEFAULT_BUSY_TIMEOUT_MS, SQLITE_STORAGE_CONFIG_PREFIX, SqliteStorageConfig,
};

#[derive(Clone)]
pub struct SqliteStorage {
    connection: Rc<RefCell<Connection>>,
    export_leases: Rc<RefCell<BTreeSet<TraceId>>>,
}

impl SqliteStorage {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open(path)?;
        configure_file_connection(&connection, None)?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            export_leases: Rc::new(RefCell::new(BTreeSet::new())),
        })
    }

    pub fn open_with_busy_timeout(
        path: &Path,
        busy_timeout: Duration,
    ) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open(path)?;
        configure_file_connection(&connection, Some(busy_timeout))?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            export_leases: Rc::new(RefCell::new(BTreeSet::new())),
        })
    }

    pub fn open_read_only(path: &Path) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        schema::validate_read_schema(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            export_leases: Rc::new(RefCell::new(BTreeSet::new())),
        })
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let connection = Connection::open_in_memory()?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            export_leases: Rc::new(RefCell::new(BTreeSet::new())),
        })
    }

    pub fn next_trace_id_seed(&self) -> Result<u64, rusqlite::Error> {
        let connection = self.connection().borrow();
        let trace_max =
            connection.query_row("SELECT COALESCE(MAX(trace_id), 0) FROM traces", [], |row| {
                row.get::<_, u64>(0)
            })?;
        let tombstone_max = connection.query_row(
            "SELECT COALESCE(MAX(trace_id), 0) FROM tombstones",
            [],
            |row| row.get::<_, u64>(0),
        )?;
        trace_max
            .max(tombstone_max)
            .checked_add(1)
            .ok_or(rusqlite::Error::InvalidQuery)
    }

    pub fn next_event_id_seed(&self) -> Result<u64, rusqlite::Error> {
        next_id_seed(&self.connection().borrow(), "events", "event_id")
    }

    pub fn next_diagnostic_id_seed(&self) -> Result<u64, rusqlite::Error> {
        next_id_seed(&self.connection().borrow(), "diagnostics", "diagnostic_id")
    }

    pub fn next_payload_segment_id_seed(&self) -> Result<u64, rusqlite::Error> {
        next_id_seed(
            &self.connection().borrow(),
            "payload_segments",
            "segment_id",
        )
    }

    pub(crate) fn connection(&self) -> &Rc<RefCell<Connection>> {
        &self.connection
    }

    pub(crate) fn export_leases(&self) -> &Rc<RefCell<BTreeSet<TraceId>>> {
        &self.export_leases
    }
}

fn configure_file_connection(
    connection: &Connection,
    busy_timeout: Option<Duration>,
) -> Result<(), rusqlite::Error> {
    if let Some(duration) = busy_timeout {
        connection.busy_timeout(duration)?;
    }
    enable_wal_journal_mode(connection)
}

fn enable_wal_journal_mode(connection: &Connection) -> Result<(), rusqlite::Error> {
    let mode = connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
        row.get::<_, String>(0)
    })?;
    if mode.eq_ignore_ascii_case("wal") {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn next_id_seed(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<u64, rusqlite::Error> {
    let query = format!("SELECT COALESCE(MAX({column}), 0) FROM {table}");
    connection
        .query_row(&query, [], |row| row.get::<_, u64>(0))?
        .checked_add(1)
        .ok_or(rusqlite::Error::InvalidQuery)
}

#[cfg(test)]
#[path = "test_cases/root_tree.rs"]
mod root_tree_tests;
#[cfg(test)]
#[path = "test_cases/upsert.rs"]
mod upsert_tests;

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
    use model_core::event::{
        DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, LossPayload,
    };
    use model_core::ids::{CollectorName, DiagnosticId, EventId, ProfileName, TraceId, TraceName};
    use model_core::process::{ProcessIdentity, ProcessMembership};
    use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
    use rusqlite::Connection;
    use semantic_action::{
        SemanticActionKind, SemanticActionLinkConfidence, SemanticActionLinkRole,
        SemanticActionReadStore, SemanticActionStatus,
    };
    use store_read_contract::diagnostics::DiagnosticReadStore;
    use store_read_contract::events::EventReadStore;
    use store_read_contract::traces::TraceReadStore;
    use store_retention_contract::cleanup::RetentionStore;
    use store_retention_contract::tombstone::TraceTombstone;
    use store_snapshot_contract::lease::SnapshotLeaseStore;
    use store_snapshot_contract::view::SnapshotStore;
    use store_tx_contract::boundary::TransactionBoundary;
    use store_write_contract::diagnostics::DiagnosticWriteStore;
    use store_write_contract::events::EventWriteStore;
    use store_write_contract::memberships::MembershipWriteStore;
    use store_write_contract::traces::TraceWriteStore;

    use crate::SqliteStorage;

    const TEST_BUSY_TIMEOUT: Duration = Duration::from_millis(1);

    #[test]
    fn initialize_rejects_legacy_schema_without_current_version() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE traces (
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
                CREATE TABLE memberships (
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
                "#,
            )
            .unwrap();
        assert!(crate::schema::initialize(&connection).is_err());
    }

    #[test]
    fn file_storage_writer_commit_succeeds_while_reader_transaction_is_open() {
        let path = temp_storage_path("wal-reader");
        cleanup_storage_files(&path);
        let mut storage = SqliteStorage::open_with_busy_timeout(&path, TEST_BUSY_TIMEOUT).unwrap();
        let reader = Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM traces", [], |row| row.get(0))
            .unwrap();

        let transaction = storage.begin().unwrap();
        let trace_id = TraceId::new(1);
        storage
            .create_trace(TraceRecord::new(
                trace_id,
                ProcessIdentity::new(100, 200, 200),
                TraceName::new("wal-reader"),
                ProfileName::new("snapshot"),
                SystemTime::UNIX_EPOCH,
            ))
            .unwrap();
        transaction.commit().unwrap();

        reader.execute_batch("ROLLBACK").unwrap();
        assert!(storage.get_trace(trace_id).is_ok());
        cleanup_storage_files(&path);
    }

    #[test]
    fn sqlite_round_trip_and_purge_are_consistent() {
        let mut storage = SqliteStorage::open_in_memory().unwrap();
        let trace_id = TraceId::new(1);
        let process = ProcessIdentity::new(100, 200, 200);
        let mut trace = TraceRecord::new(
            trace_id,
            process.clone(),
            TraceName::new("demo"),
            ProfileName::new("snapshot"),
            SystemTime::UNIX_EPOCH,
        );
        trace.lifecycle_state = TraceLifecycleState::Completed;
        trace.health = TraceHealth::Degraded;
        storage.create_trace(trace.clone()).unwrap();
        storage
            .upsert_membership(ProcessMembership::root(
                trace_id,
                process.clone(),
                SystemTime::UNIX_EPOCH,
            ))
            .unwrap();
        storage
            .append_event(DomainEvent::new(
                EventEnvelope {
                    event_id: EventId::new(1),
                    trace_id,
                    observed_at: SystemTime::UNIX_EPOCH,
                    process: process.clone(),
                    collector: CollectorName::new("ebpf"),
                    kind: EventKind::Loss,
                    flags: EventFlags::clean(),
                },
                EventPayload::Loss(LossPayload {
                    reason: "bootstrap_gap".to_string(),
                    fatal: false,
                }),
            ))
            .unwrap();
        storage
            .append_diagnostic(DiagnosticRecord::new(
                DiagnosticId::new(1),
                Some(trace_id),
                DiagnosticKind::BootstrapGap,
                DiagnosticSeverity::Warning,
                SystemTime::UNIX_EPOCH,
                "gap",
            ))
            .unwrap();

        let lease = storage.acquire_export_lease(trace_id).unwrap();
        let snapshot = storage.read_snapshot(&lease).unwrap();
        assert_eq!(snapshot.trace.trace_id, trace_id);
        assert_eq!(snapshot.memberships.len(), 1);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.diagnostics.len(), 1);
        storage.release_export_lease(lease).unwrap();

        storage
            .purge_trace(
                trace_id,
                TraceTombstone {
                    trace_id,
                    lifecycle_state: TraceLifecycleState::Completed,
                    health: TraceHealth::Degraded,
                    cleaned_at: SystemTime::UNIX_EPOCH,
                    cleanup_reason: "test".to_string(),
                },
            )
            .unwrap();

        assert!(storage.get_trace(trace_id).is_err());
        assert!(storage.list_events(trace_id).is_err());
        assert!(storage.list_diagnostics(trace_id).is_err());
    }

    #[test]
    fn open_migrates_v6_semantic_action_strings_to_codebook_columns() {
        let path = temp_storage_path("semantic-codebook-v6");
        cleanup_storage_files(&path);
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE semantic_actions (
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
                    CREATE TABLE semantic_action_evidence (
                        action_id TEXT NOT NULL,
                        evidence_order INTEGER NOT NULL,
                        kind TEXT NOT NULL,
                        evidence_id INTEGER NOT NULL,
                        role TEXT NOT NULL,
                        PRIMARY KEY (action_id, evidence_order)
                    );
                    CREATE TABLE semantic_action_links (
                        trace_id INTEGER NOT NULL,
                        parent_action_id TEXT NOT NULL,
                        child_action_id TEXT NOT NULL,
                        role TEXT NOT NULL,
                        confidence TEXT NOT NULL,
                        valid INTEGER NOT NULL DEFAULT 1,
                        attributes TEXT NOT NULL,
                        PRIMARY KEY (trace_id, parent_action_id, child_action_id, role)
                    );
                    CREATE TABLE semantic_action_link_evidence (
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
                    CREATE TABLE file_observation_paths (
                        trace_id INTEGER NOT NULL,
                        action_id TEXT NOT NULL,
                        path_order INTEGER NOT NULL,
                        path TEXT NOT NULL,
                        PRIMARY KEY (trace_id, action_id, path)
                    );
                    CREATE TABLE file_path_set_action_refs (
                        trace_id INTEGER NOT NULL,
                        action_id TEXT NOT NULL,
                        path_set_id TEXT NOT NULL,
                        PRIMARY KEY (trace_id, action_id)
                    );
                    CREATE TABLE llm_request_manifests (
                        manifest_id INTEGER PRIMARY KEY,
                        trace_id INTEGER NOT NULL,
                        action_id TEXT NOT NULL,
                        format_version INTEGER NOT NULL,
                        canonical_body_hash BLOB NOT NULL,
                        canonical_body_bytes INTEGER NOT NULL,
                        skeleton_json TEXT NOT NULL,
                        UNIQUE (trace_id, action_id)
                    );
                    INSERT INTO semantic_actions (
                        action_id, trace_id, kind, title, start_time, end_time,
                        process_pid, process_task_id, process_start_ticks,
                        process_generation, status, completeness, confidence_millis, attributes
                    ) VALUES
                        ('parent', 1, 'command.invocation', 'command', 1, 2, 100, 10, 11, 0, 'success', 'complete', NULL, ''),
                        ('child', 1, 'llm.call', 'call', 2, 3, 100, 10, 11, 0, 'success', 'complete', NULL, '');
                    INSERT INTO semantic_action_evidence (
                        action_id, evidence_order, kind, evidence_id, role
                    ) VALUES ('child', 0, 'payload_segment', 42, 'llm.payload');
                    INSERT INTO semantic_action_links (
                        trace_id, parent_action_id, child_action_id, role, confidence, valid, attributes
                    ) VALUES (1, 'parent', 'child', 'command.contains_llm_call', 'derived', 1, '');
                    INSERT INTO semantic_action_link_evidence (
                        trace_id, parent_action_id, child_action_id, role,
                        evidence_order, kind, evidence_id, evidence_role
                    ) VALUES (1, 'parent', 'child', 'command.contains_llm_call', 0, 'payload_segment', 43, 'llm.payload');
                    INSERT INTO file_observation_paths (
                        trace_id, action_id, path_order, path
                    ) VALUES (1, 'child', 0, '/tmp/input');
                    INSERT INTO file_path_set_action_refs (
                        trace_id, action_id, path_set_id
                    ) VALUES (1, 'child', 'path-set-1');
                    INSERT INTO llm_request_manifests (
                        manifest_id, trace_id, action_id, format_version,
                        canonical_body_hash, canonical_body_bytes, skeleton_json
                    ) VALUES (7, 1, 'child', 1, X'001122', 3, '{"ok":true}');
                    PRAGMA user_version = 6;
                    "#,
                )
                .unwrap();
        }

        let storage = SqliteStorage::open(&path).unwrap();
        let version = storage
            .connection()
            .borrow()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(version, 7);
        let legacy_kind_columns = storage
            .connection()
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('semantic_actions')
                 WHERE name = 'kind'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(legacy_kind_columns, 0);

        let actions = storage.list_semantic_actions(TraceId::new(1)).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].kind, SemanticActionKind::CommandInvocation);
        assert_eq!(actions[1].kind, SemanticActionKind::LlmCall);
        assert_eq!(actions[1].status, SemanticActionStatus::Success);
        assert_eq!(actions[1].evidence[0].id, 42);

        let links = storage.list_semantic_action_links(TraceId::new(1)).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].role,
            SemanticActionLinkRole::CommandContainsLlmCall
        );
        assert_eq!(links[0].confidence, SemanticActionLinkConfidence::Derived);
        assert_eq!(links[0].evidence[0].id, 43);

        for table in [
            "file_observation_paths",
            "file_path_set_action_refs",
            "llm_request_manifests",
        ] {
            let legacy_action_id_columns = storage
                .connection()
                .borrow()
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM pragma_table_info('{table}')
                         WHERE name = 'action_id'"
                    ),
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(legacy_action_id_columns, 0);
        }

        let migrated_path_action = storage
            .connection()
            .borrow()
            .query_row(
                "SELECT ids.action_id
                 FROM file_observation_paths paths
                 JOIN semantic_action_ids ids
                   ON ids.action_key = paths.action_key
                 WHERE paths.path = '/tmp/input'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(migrated_path_action, "child");

        let migrated_path_set_action = storage
            .connection()
            .borrow()
            .query_row(
                "SELECT ids.action_id
                 FROM file_path_set_action_refs refs
                 JOIN semantic_action_ids ids
                   ON ids.action_key = refs.action_key
                 WHERE refs.path_set_id = 'path-set-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(migrated_path_set_action, "child");

        let migrated_manifest = storage
            .connection()
            .borrow()
            .query_row(
                "SELECT manifest.manifest_id, ids.action_id, manifest.skeleton_json
                 FROM llm_request_manifests manifest
                 JOIN semantic_action_ids ids
                   ON ids.action_key = manifest.action_key",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            migrated_manifest,
            (7, "child".to_string(), "{\"ok\":true}".to_string())
        );
        cleanup_storage_files(&path);
    }

    fn temp_storage_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "actrail-sqlite-storage-{name}-{}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup_storage_files(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }
}

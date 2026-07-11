//! SQLite-backed storage adapter.

pub mod backend;
pub mod config;
pub mod query;
pub mod records;
pub mod retention;
pub mod schema;
pub mod semantic_actions;
pub mod transaction;
pub mod writer;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use model_core::ids::TraceId;
use model_core::process::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessIdentity,
    ProcessRecord, ProcessResolutionState,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};

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

    pub fn reserve_process_id_block(&mut self, count: u64) -> Result<(u64, u64), rusqlite::Error> {
        if count == 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut connection = self.connection().borrow_mut();
        let transaction = connection.transaction()?;
        let start = transaction.query_row(
            "SELECT next_process_id FROM process_id_sequence WHERE singleton = 1",
            [],
            |row| row.get::<_, u64>(0),
        )?;
        let end = start
            .checked_add(count)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        transaction.execute(
            "UPDATE process_id_sequence SET next_process_id = ?1 WHERE singleton = 1",
            [end],
        )?;
        transaction.commit()?;
        Ok((start, end))
    }

    pub fn upsert_process_record(&mut self, record: &ProcessRecord) -> Result<(), rusqlite::Error> {
        let mut connection = self.connection().borrow_mut();
        if !connection.is_autocommit() {
            return Self::upsert_process_record_on(&connection, record);
        }
        let transaction = connection.transaction()?;
        Self::upsert_process_record_on(&transaction, record)?;
        transaction.commit()
    }

    fn upsert_process_record_on(
        connection: &Connection,
        record: &ProcessRecord,
    ) -> Result<(), rusqlite::Error> {
        let host = record.host.as_ref();
        connection.execute(
            "INSERT INTO processes (
                process_id, host_pid, host_task_id, host_start_ticks,
                host_start_boottime_ns, resolution_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(process_id) DO UPDATE SET
                host_pid = excluded.host_pid,
                host_task_id = excluded.host_task_id,
                host_start_ticks = excluded.host_start_ticks,
                host_start_boottime_ns = excluded.host_start_boottime_ns,
                resolution_state = excluded.resolution_state",
            rusqlite::params![
                record.identity.get(),
                host.map(|value| value.pid),
                host.and_then(|value| value.task_id),
                host.map(|value| value.start_time_ticks),
                host.and_then(|value| value.start_boottime_ns),
                ProcessRecordCodec::resolution_state_name(record.resolution_state),
            ],
        )?;
        for namespace in &record.namespaces {
            connection.execute(
                "INSERT OR IGNORE INTO process_namespace_aliases (
                    process_id, pid_namespace, namespace_pid, namespace_start_ticks
                 ) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    record.identity.get(),
                    namespace.pid_namespace.as_str(),
                    namespace.pid,
                    namespace.start_time_ticks,
                ],
            )?;
        }
        Ok(())
    }

    pub fn get_process_record(
        &self,
        identity: ProcessIdentity,
    ) -> Result<Option<ProcessRecord>, rusqlite::Error> {
        let connection = self.connection().borrow();
        let mut record = connection
            .query_row(
                "SELECT process_id, host_pid, host_task_id, host_start_ticks,
                        host_start_boottime_ns, resolution_state
                 FROM processes WHERE process_id = ?1",
                [identity.get()],
                ProcessRecordCodec::from_row,
            )
            .optional()?;
        if let Some(record) = &mut record {
            ProcessRecordCodec::load_namespaces(&connection, record)?;
        }
        Ok(record)
    }

    pub fn list_process_records(&self) -> Result<Vec<ProcessRecord>, rusqlite::Error> {
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(
            "SELECT process_id, host_pid, host_task_id, host_start_ticks,
                    host_start_boottime_ns, resolution_state
             FROM processes ORDER BY process_id",
        )?;
        let rows = statement.query_map([], ProcessRecordCodec::from_row)?;
        let mut records = rows.collect::<Result<Vec<_>, _>>()?;
        for record in &mut records {
            ProcessRecordCodec::load_namespaces(&connection, record)?;
        }
        Ok(records)
    }

    pub(crate) fn connection(&self) -> &Rc<RefCell<Connection>> {
        &self.connection
    }

    pub(crate) fn export_leases(&self) -> &Rc<RefCell<BTreeSet<TraceId>>> {
        &self.export_leases
    }
}

struct ProcessRecordCodec;

impl ProcessRecordCodec {
    fn from_row(row: &Row<'_>) -> Result<ProcessRecord, rusqlite::Error> {
        let identity = ProcessIdentity::new(row.get(0)?);
        let host_pid = row.get::<_, Option<u32>>(1)?;
        let host = host_pid.map(|pid| HostProcessCoordinates {
            pid,
            task_id: row.get(2).expect("host task id column"),
            start_time_ticks: row
                .get::<_, Option<u64>>(3)
                .expect("host start ticks column")
                .unwrap_or(0),
            start_boottime_ns: row.get(4).expect("host boot time column"),
        });
        Ok(ProcessRecord {
            identity,
            host,
            namespaces: BTreeSet::new(),
            resolution_state: Self::parse_resolution_state(row.get::<_, String>(5)?.as_str())?,
        })
    }

    fn load_namespaces(
        connection: &Connection,
        record: &mut ProcessRecord,
    ) -> Result<(), rusqlite::Error> {
        let mut aliases = connection.prepare(
            "SELECT pid_namespace, namespace_pid, namespace_start_ticks
         FROM process_namespace_aliases WHERE process_id = ?1",
        )?;
        let rows = aliases.query_map([record.identity.get()], |row| {
            Ok(NamespaceProcessCoordinates::new(
                NamespaceIdentity::new(row.get::<_, String>(0)?),
                row.get(1)?,
                row.get(2)?,
            ))
        })?;
        record.namespaces = rows.collect::<Result<_, _>>()?;
        Ok(())
    }

    fn resolution_state_name(state: ProcessResolutionState) -> &'static str {
        match state {
            ProcessResolutionState::Provisional => "provisional",
            ProcessResolutionState::Resolved => "resolved",
            ProcessResolutionState::Conflicted => "conflicted",
        }
    }

    fn parse_resolution_state(value: &str) -> Result<ProcessResolutionState, rusqlite::Error> {
        match value {
            "provisional" => Ok(ProcessResolutionState::Provisional),
            "resolved" => Ok(ProcessResolutionState::Resolved),
            "conflicted" => Ok(ProcessResolutionState::Conflicted),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
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
                ProcessIdentity::new(200),
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
        let process = ProcessIdentity::new(200);
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
    fn open_rejects_v6_semantic_action_schema() {
        let path = temp_storage_path("semantic-codebook-v6");
        cleanup_storage_files(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE legacy_schema_marker (id INTEGER PRIMARY KEY);
                 PRAGMA user_version = 6;",
            )
            .unwrap();
        drop(connection);

        assert!(SqliteStorage::open(&path).is_err());
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

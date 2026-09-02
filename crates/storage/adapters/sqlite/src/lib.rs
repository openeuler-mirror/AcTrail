//! SQLite-backed storage adapter.

pub mod alerts;
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

use model_core::process::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessIdentity,
    ProcessRecord, ProcessResolutionState,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};

pub use config::{
    EventRecordLayout, SQLITE_DEFAULT_BUSY_TIMEOUT_MS,
    SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES,
    SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES,
    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS,
    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES,
    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL, SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS,
    SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES, SQLITE_STORAGE_CONFIG_PREFIX,
    SqliteStorageConfig,
};
pub use semantic_actions::storage_meta::ColdFieldCompression;

#[derive(Clone)]
pub struct SqliteStorage {
    connection: Rc<RefCell<Connection>>,
    trace_leases: Rc<RefCell<crate::query::TraceLeaseRegistry>>,
    cold_field_compression: ColdFieldCompression,
    event_payload_dictionary: Rc<RefCell<crate::records::EventPayloadDictionary>>,
    event_path_dictionary: Rc<RefCell<crate::records::PathInterner>>,
    event_record_blocks: Rc<RefCell<crate::records::EventRecordBlockWriter>>,
}

impl SqliteStorage {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        Self::open_with_compression(path, None, ColdFieldCompression::DEFAULT)
    }

    pub fn open_with_busy_timeout(
        path: &Path,
        busy_timeout: Duration,
    ) -> Result<Self, rusqlite::Error> {
        Self::open_with_compression(path, Some(busy_timeout), ColdFieldCompression::DEFAULT)
    }

    pub fn open_with_compression(
        path: &Path,
        busy_timeout: Option<Duration>,
        cold_field_compression: ColdFieldCompression,
    ) -> Result<Self, rusqlite::Error> {
        Self::open_with_options(
            path,
            busy_timeout,
            cold_field_compression,
            SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES,
            SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES,
            EventRecordLayout::Rows,
            SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS,
            SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES,
            SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL,
        )
    }

    pub fn open_with_options(
        path: &Path,
        busy_timeout: Option<Duration>,
        cold_field_compression: ColdFieldCompression,
        event_payload_dictionary_cache_bytes: usize,
        event_path_dictionary_cache_bytes: usize,
        event_record_layout: EventRecordLayout,
        event_record_block_max_events: usize,
        event_record_block_max_uncompressed_bytes: usize,
        event_record_block_zstd_level: i32,
    ) -> Result<Self, rusqlite::Error> {
        validate_event_record_options(
            event_record_block_max_events,
            event_record_block_max_uncompressed_bytes,
            event_record_block_zstd_level,
        )?;
        let connection = Connection::open(path)?;
        configure_file_connection(&connection, busy_timeout)?;
        schema::initialize(&connection)?;
        let event_id_high_water = read_event_id_high_water(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            trace_leases: Rc::new(RefCell::new(crate::query::TraceLeaseRegistry::new())),
            cold_field_compression,
            event_payload_dictionary: Rc::new(RefCell::new(
                crate::records::EventPayloadDictionary::new(event_payload_dictionary_cache_bytes),
            )),
            event_path_dictionary: Rc::new(RefCell::new(crate::records::PathInterner::new(
                event_path_dictionary_cache_bytes,
            ))),
            event_record_blocks: Rc::new(RefCell::new(
                crate::records::EventRecordBlockWriter::new(
                    event_record_layout,
                    event_record_block_max_events,
                    event_record_block_max_uncompressed_bytes,
                    event_record_block_zstd_level,
                    event_id_high_water,
                ),
            )),
        })
    }

    pub fn open_read_only(path: &Path) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        schema::validate_read_schema(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            trace_leases: Rc::new(RefCell::new(crate::query::TraceLeaseRegistry::new())),
            cold_field_compression: ColdFieldCompression::DEFAULT,
            event_payload_dictionary: Rc::new(RefCell::new(
                crate::records::EventPayloadDictionary::new(0),
            )),
            event_path_dictionary: Rc::new(RefCell::new(crate::records::PathInterner::new(0))),
            event_record_blocks: Rc::new(RefCell::new(
                crate::records::EventRecordBlockWriter::new(
                    EventRecordLayout::Rows,
                    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS,
                    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES,
                    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL,
                    0,
                ),
            )),
        })
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let connection = Connection::open_in_memory()?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            trace_leases: Rc::new(RefCell::new(crate::query::TraceLeaseRegistry::new())),
            cold_field_compression: ColdFieldCompression::DEFAULT,
            event_payload_dictionary: Rc::new(RefCell::new(
                crate::records::EventPayloadDictionary::new(
                    SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES,
                ),
            )),
            event_path_dictionary: Rc::new(RefCell::new(crate::records::PathInterner::new(
                SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES,
            ))),
            event_record_blocks: Rc::new(RefCell::new(
                crate::records::EventRecordBlockWriter::new(
                    EventRecordLayout::Rows,
                    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS,
                    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES,
                    SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL,
                    0,
                ),
            )),
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
        let connection = self.connection().borrow();
        read_event_id_high_water(&connection)?
            .checked_add(1)
            .ok_or(rusqlite::Error::InvalidQuery)
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

    pub(crate) fn trace_leases(&self) -> &Rc<RefCell<crate::query::TraceLeaseRegistry>> {
        &self.trace_leases
    }

    pub(crate) fn event_payload_dictionary(
        &self,
    ) -> &Rc<RefCell<crate::records::EventPayloadDictionary>> {
        &self.event_payload_dictionary
    }

    pub(crate) fn event_path_dictionary(&self) -> &Rc<RefCell<crate::records::PathInterner>> {
        &self.event_path_dictionary
    }

    pub(crate) fn event_record_blocks(
        &self,
    ) -> &Rc<RefCell<crate::records::EventRecordBlockWriter>> {
        &self.event_record_blocks
    }
}

fn read_event_id_high_water(connection: &Connection) -> Result<u64, rusqlite::Error> {
    connection.query_row(
        "SELECT last_event_id FROM event_id_high_water WHERE singleton = 1",
        [],
        |row| row.get::<_, u64>(0),
    )
}

fn validate_event_record_options(
    max_events: usize,
    max_uncompressed_bytes: usize,
    zstd_level: i32,
) -> Result<(), rusqlite::Error> {
    if max_events == 0
        || max_events > SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS
        || max_uncompressed_bytes == 0
        || max_uncompressed_bytes > SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES
        || !(-7..=22).contains(&zstd_level)
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "invalid event record block limits".to_string(),
        ));
    }
    Ok(())
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

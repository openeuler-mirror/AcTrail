//! SQLite-backed storage adapter.

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

use model_core::ids::TraceId;
use rusqlite::{Connection, OpenFlags};

#[derive(Clone)]
pub struct SqliteStorage {
    connection: Rc<RefCell<Connection>>,
    export_leases: Rc<RefCell<BTreeSet<TraceId>>>,
}

impl SqliteStorage {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open(path)?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Rc::new(RefCell::new(connection)),
            export_leases: Rc::new(RefCell::new(BTreeSet::new())),
        })
    }

    pub fn open_read_only(path: &Path) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
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
mod tests {
    use std::time::SystemTime;

    use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
    use model_core::event::{
        DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, LossPayload,
    };
    use model_core::ids::{CollectorName, DiagnosticId, EventId, ProfileName, TraceId, TraceName};
    use model_core::process::{ProcessIdentity, ProcessMembership};
    use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
    use store_read_contract::diagnostics::DiagnosticReadStore;
    use store_read_contract::events::EventReadStore;
    use store_read_contract::traces::TraceReadStore;
    use store_retention_contract::cleanup::RetentionStore;
    use store_retention_contract::tombstone::TraceTombstone;
    use store_snapshot_contract::lease::SnapshotLeaseStore;
    use store_snapshot_contract::view::SnapshotStore;
    use store_write_contract::diagnostics::DiagnosticWriteStore;
    use store_write_contract::events::EventWriteStore;
    use store_write_contract::memberships::MembershipWriteStore;
    use store_write_contract::traces::TraceWriteStore;

    use crate::SqliteStorage;

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
            .upsert_membership(ProcessMembership::root(trace_id, process.clone()))
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
}

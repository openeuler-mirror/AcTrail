//! Unified storage backend implementation for SQLite.

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::{ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
use semantic_action::{SemanticAction, SemanticActionLink};
use storage_core::{
    ExportLease, PayloadSegmentQuery, RetentionCandidate, SemanticActionChildPage,
    SemanticActionChildPageQuery, SemanticActionChildRow, SemanticActionDisplayRootChildPage,
    SemanticActionDisplayRootChildRow, SemanticActionSummary, SnapshotView, StorageBackend,
    StorageError, StorageTransaction, TraceFilter, TraceTombstone,
};
use store_read_contract::diagnostics::DiagnosticReadStore;
use store_read_contract::events::EventReadStore;
use store_read_contract::payloads::PayloadReadStore;
use store_read_contract::traces::TraceReadStore;
use store_retention_contract::cleanup::RetentionStore;
use store_snapshot_contract::lease::SnapshotLeaseStore;
use store_snapshot_contract::view::SnapshotStore;
use store_tx_contract::boundary::TransactionBoundary;
use store_write_contract::diagnostics::DiagnosticWriteStore;
use store_write_contract::events::EventWriteStore;
use store_write_contract::memberships::MembershipWriteStore;
use store_write_contract::payloads::PayloadWriteStore;
use store_write_contract::traces::TraceWriteStore;

use crate::SqliteStorage;

struct LegacyTransaction {
    inner: Box<dyn store_tx_contract::boundary::StorageTransaction>,
}

impl StorageTransaction for LegacyTransaction {
    fn commit(self: Box<Self>) -> Result<(), StorageError> {
        self.inner.commit().map_err(StorageError::from)
    }

    fn rollback(self: Box<Self>) -> Result<(), StorageError> {
        self.inner.rollback().map_err(StorageError::from)
    }
}

impl StorageBackend for SqliteStorage {
    fn next_trace_id_seed(&self) -> Result<u64, StorageError> {
        SqliteStorage::next_trace_id_seed(self)
            .map_err(|error| StorageError::new("trace_id_seed", error.to_string()))
    }

    fn next_event_id_seed(&self) -> Result<u64, StorageError> {
        SqliteStorage::next_event_id_seed(self)
            .map_err(|error| StorageError::new("event_id_seed", error.to_string()))
    }

    fn next_diagnostic_id_seed(&self) -> Result<u64, StorageError> {
        SqliteStorage::next_diagnostic_id_seed(self)
            .map_err(|error| StorageError::new("diagnostic_id_seed", error.to_string()))
    }

    fn next_payload_segment_id_seed(&self) -> Result<u64, StorageError> {
        SqliteStorage::next_payload_segment_id_seed(self)
            .map_err(|error| StorageError::new("payload_segment_id_seed", error.to_string()))
    }

    fn begin(&mut self) -> Result<Box<dyn StorageTransaction>, StorageError> {
        TransactionBoundary::begin(self)
            .map(|inner| Box::new(LegacyTransaction { inner }) as Box<dyn StorageTransaction>)
            .map_err(StorageError::from)
    }

    fn create_trace(&mut self, trace: TraceRecord) -> Result<(), StorageError> {
        TraceWriteStore::create_trace(self, trace).map_err(StorageError::from)
    }

    fn update_trace_lifecycle(
        &mut self,
        trace_id: TraceId,
        lifecycle_state: TraceLifecycleState,
    ) -> Result<(), StorageError> {
        TraceWriteStore::update_trace_lifecycle(self, trace_id, lifecycle_state)
            .map_err(StorageError::from)
    }

    fn update_trace_health(
        &mut self,
        trace_id: TraceId,
        health: TraceHealth,
    ) -> Result<(), StorageError> {
        TraceWriteStore::update_trace_health(self, trace_id, health).map_err(StorageError::from)
    }

    fn get_trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, StorageError> {
        TraceReadStore::get_trace(self, trace_id).map_err(StorageError::from)
    }

    fn list_traces(&self, filter: &TraceFilter) -> Result<Vec<TraceRecord>, StorageError> {
        TraceReadStore::list_traces(self, filter).map_err(StorageError::from)
    }

    fn upsert_membership(&mut self, membership: ProcessMembership) -> Result<(), StorageError> {
        MembershipWriteStore::upsert_membership(self, membership).map_err(StorageError::from)
    }

    fn trace_memberships(&self, trace_id: TraceId) -> Result<Vec<ProcessMembership>, StorageError> {
        SqliteStorage::trace_memberships(self, trace_id).map_err(StorageError::from)
    }

    fn append_event(&mut self, event: DomainEvent) -> Result<(), StorageError> {
        EventWriteStore::append_event(self, event).map_err(StorageError::from)
    }

    fn list_events(&self, trace_id: TraceId) -> Result<Vec<DomainEvent>, StorageError> {
        EventReadStore::list_events(self, trace_id).map_err(StorageError::from)
    }

    fn count_events_by_variant(
        &self,
        trace_id: TraceId,
    ) -> Result<std::collections::BTreeMap<String, usize>, StorageError> {
        SqliteStorage::count_events_by_variant(self, trace_id).map_err(StorageError::from)
    }

    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), StorageError> {
        PayloadWriteStore::append_payload_segment(self, segment).map_err(StorageError::from)
    }

    fn list_payload_segments(
        &self,
        trace_id: TraceId,
        query: PayloadSegmentQuery,
    ) -> Result<Vec<PayloadSegment>, StorageError> {
        PayloadReadStore::list_payload_segments(self, trace_id, query).map_err(StorageError::from)
    }

    fn count_payload_segments(&self, trace_id: TraceId) -> Result<usize, StorageError> {
        SqliteStorage::count_payload_segments(self, trace_id).map_err(StorageError::from)
    }

    fn retained_payload_bytes(&self, trace_id: TraceId) -> Result<u64, StorageError> {
        PayloadReadStore::retained_payload_bytes(self, trace_id).map_err(StorageError::from)
    }

    fn append_diagnostic(&mut self, diagnostic: DiagnosticRecord) -> Result<(), StorageError> {
        DiagnosticWriteStore::append_diagnostic(self, diagnostic).map_err(StorageError::from)
    }

    fn list_diagnostics(&self, trace_id: TraceId) -> Result<Vec<DiagnosticRecord>, StorageError> {
        DiagnosticReadStore::list_diagnostics(self, trace_id).map_err(StorageError::from)
    }

    fn upsert_semantic_action(&mut self, action: SemanticAction) -> Result<(), StorageError> {
        semantic_action::SemanticActionWriteStore::upsert_semantic_action(self, action)
            .map_err(StorageError::from)
    }

    fn upsert_semantic_action_link(
        &mut self,
        link: SemanticActionLink,
    ) -> Result<(), StorageError> {
        semantic_action::SemanticActionWriteStore::upsert_semantic_action_link(self, link)
            .map_err(StorageError::from)
    }

    fn list_semantic_actions(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticAction>, StorageError> {
        semantic_action::SemanticActionReadStore::list_semantic_actions(self, trace_id)
            .map_err(StorageError::from)
    }

    fn list_semantic_action_links(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticActionLink>, StorageError> {
        semantic_action::SemanticActionReadStore::list_semantic_action_links(self, trace_id)
            .map_err(StorageError::from)
    }

    fn semantic_actions_matching_kinds(
        &self,
        trace_id: TraceId,
        kinds: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError> {
        SqliteStorage::semantic_actions_matching_kinds(self, trace_id, kinds)
            .map_err(StorageError::from)
    }

    fn semantic_action_summary(
        &self,
        trace_id: TraceId,
    ) -> Result<SemanticActionSummary, StorageError> {
        SqliteStorage::semantic_action_summary(self, trace_id)
            .map(|summary| SemanticActionSummary {
                actions: summary.actions,
                links: summary.links,
                roots: summary.roots,
            })
            .map_err(StorageError::from)
    }

    fn observed_agent_semantic_action(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<SemanticAction>, StorageError> {
        SqliteStorage::observed_agent_semantic_action(self, trace_id).map_err(StorageError::from)
    }

    fn semantic_action_children(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, StorageError> {
        SqliteStorage::semantic_action_children(
            self,
            trace_id,
            parent_action_id,
            roles,
            child_roles,
        )
        .map(convert_child_rows)
        .map_err(StorageError::from)
    }

    fn semantic_action_children_page(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionChildPage, StorageError> {
        SqliteStorage::semantic_action_children_page(
            self,
            trace_id,
            parent_action_id,
            roles,
            child_roles,
            crate::semantic_actions::SemanticActionChildPageQuery {
                offset: page.offset,
                limit: page.limit,
            },
        )
        .map(|page| SemanticActionChildPage {
            rows: convert_child_rows(page.rows),
            total_count: page.total_count,
        })
        .map_err(StorageError::from)
    }

    fn semantic_action_display_root_children_page(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
        root_link_roles: &[&str],
        page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionDisplayRootChildPage, StorageError> {
        SqliteStorage::semantic_action_display_root_children_page(
            self,
            trace_id,
            display_parent_roles,
            root_link_roles,
            crate::semantic_actions::SemanticActionChildPageQuery {
                offset: page.offset,
                limit: page.limit,
            },
        )
        .map(|page| SemanticActionDisplayRootChildPage {
            rows: convert_display_root_child_rows(page.rows),
            total_count: page.total_count,
        })
        .map_err(StorageError::from)
    }

    fn semantic_action_display_root_child_count(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
    ) -> Result<usize, StorageError> {
        SqliteStorage::semantic_action_display_root_child_count(
            self,
            trace_id,
            display_parent_roles,
        )
        .map_err(StorageError::from)
    }

    fn semantic_action_children_matching_kinds(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        child_kinds: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, StorageError> {
        SqliteStorage::semantic_action_children_matching_kinds(
            self,
            trace_id,
            parent_action_id,
            roles,
            child_roles,
            child_kinds,
        )
        .map(convert_child_rows)
        .map_err(StorageError::from)
    }

    fn semantic_action_by_id(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Option<SemanticAction>, StorageError> {
        SqliteStorage::semantic_action_by_id(self, trace_id, action_id).map_err(StorageError::from)
    }

    fn semantic_action_command_fallback_children(
        &self,
        trace_id: TraceId,
        command: &SemanticAction,
        display_parent_roles: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError> {
        SqliteStorage::semantic_action_command_fallback_children(
            self,
            trace_id,
            command,
            display_parent_roles,
        )
        .map_err(StorageError::from)
    }

    fn semantic_action_for_process_kind(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        kind: &str,
    ) -> Result<Option<SemanticAction>, StorageError> {
        SqliteStorage::semantic_action_for_process_kind(self, trace_id, process, kind)
            .map_err(StorageError::from)
    }

    fn semantic_action_child_count(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
    ) -> Result<usize, StorageError> {
        SqliteStorage::semantic_action_child_count(self, trace_id, parent_action_id, roles)
            .map_err(StorageError::from)
    }

    fn acquire_export_lease(&mut self, trace_id: TraceId) -> Result<ExportLease, StorageError> {
        SnapshotLeaseStore::acquire_export_lease(self, trace_id).map_err(StorageError::from)
    }

    fn release_export_lease(&mut self, lease: ExportLease) -> Result<(), StorageError> {
        SnapshotLeaseStore::release_export_lease(self, lease).map_err(StorageError::from)
    }

    fn read_snapshot(&self, lease: &ExportLease) -> Result<SnapshotView, StorageError> {
        SnapshotStore::read_snapshot(self, lease).map_err(StorageError::from)
    }

    fn list_terminal_candidates(&self) -> Result<Vec<RetentionCandidate>, StorageError> {
        RetentionStore::list_terminal_candidates(self).map_err(StorageError::from)
    }

    fn purge_trace(
        &mut self,
        trace_id: TraceId,
        tombstone: TraceTombstone,
    ) -> Result<(), StorageError> {
        RetentionStore::purge_trace(self, trace_id, tombstone).map_err(StorageError::from)
    }
}

fn convert_child_rows(
    rows: Vec<crate::semantic_actions::SemanticActionChildRow>,
) -> Vec<SemanticActionChildRow> {
    rows.into_iter()
        .map(|row| SemanticActionChildRow {
            link: row.link,
            action: row.action,
            child_count: row.child_count,
        })
        .collect()
}

fn convert_display_root_child_rows(
    rows: Vec<crate::semantic_actions::SemanticActionDisplayRootChildRow>,
) -> Vec<SemanticActionDisplayRootChildRow> {
    rows.into_iter()
        .map(|row| SemanticActionDisplayRootChildRow {
            root_link: row.root_link,
            action: row.action,
            child_count: row.child_count,
        })
        .collect()
}

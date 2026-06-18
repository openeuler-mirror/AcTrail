//! Unified storage backend trait.

use std::collections::BTreeMap;

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::{ProcessIdentity, ProcessMembership};
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
use semantic_action::{
    FileObservationPath, FilePathSetPathPage, FilePathSetWrite, SemanticAction, SemanticActionLink,
};

use crate::{
    ExportLease, PayloadSegmentQuery, RetentionCandidate, SnapshotView, StorageError,
    StorageTransaction, TraceFilter, TraceTombstone,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageOpenMode {
    ReadWrite,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionChildRow {
    pub link: SemanticActionLink,
    pub action: SemanticAction,
    pub child_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionDisplayRootChildRow {
    pub root_link: Option<SemanticActionLink>,
    pub action: SemanticAction,
    pub child_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticActionSummary {
    pub actions: usize,
    pub links: usize,
    pub roots: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionChildPage {
    pub rows: Vec<SemanticActionChildRow>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionDisplayRootChildPage {
    pub rows: Vec<SemanticActionDisplayRootChildRow>,
    pub total_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticActionChildPageQuery {
    pub offset: usize,
    pub limit: usize,
}

pub trait StorageBackend {
    fn next_trace_id_seed(&self) -> Result<u64, StorageError>;
    fn next_event_id_seed(&self) -> Result<u64, StorageError>;
    fn next_diagnostic_id_seed(&self) -> Result<u64, StorageError>;
    fn next_payload_segment_id_seed(&self) -> Result<u64, StorageError>;

    fn begin(&mut self) -> Result<Box<dyn StorageTransaction>, StorageError>;

    fn create_trace(&mut self, trace: TraceRecord) -> Result<(), StorageError>;
    fn update_trace_lifecycle(
        &mut self,
        trace_id: TraceId,
        lifecycle_state: TraceLifecycleState,
    ) -> Result<(), StorageError>;
    fn update_trace_health(
        &mut self,
        trace_id: TraceId,
        health: TraceHealth,
    ) -> Result<(), StorageError>;
    fn get_trace(&self, trace_id: TraceId) -> Result<Option<TraceRecord>, StorageError>;
    fn list_traces(&self, filter: &TraceFilter) -> Result<Vec<TraceRecord>, StorageError>;

    fn upsert_membership(&mut self, membership: ProcessMembership) -> Result<(), StorageError>;
    fn trace_memberships(&self, trace_id: TraceId) -> Result<Vec<ProcessMembership>, StorageError>;

    fn append_event(&mut self, event: DomainEvent) -> Result<(), StorageError>;
    fn list_events(&self, trace_id: TraceId) -> Result<Vec<DomainEvent>, StorageError>;
    fn count_events_by_variant(
        &self,
        trace_id: TraceId,
    ) -> Result<BTreeMap<String, usize>, StorageError>;

    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), StorageError>;
    fn list_payload_segments(
        &self,
        trace_id: TraceId,
        query: PayloadSegmentQuery,
    ) -> Result<Vec<PayloadSegment>, StorageError>;
    fn count_payload_segments(&self, trace_id: TraceId) -> Result<usize, StorageError>;
    fn retained_payload_bytes(&self, trace_id: TraceId) -> Result<u64, StorageError>;

    fn append_diagnostic(&mut self, diagnostic: DiagnosticRecord) -> Result<(), StorageError>;
    fn list_diagnostics(&self, trace_id: TraceId) -> Result<Vec<DiagnosticRecord>, StorageError>;

    fn upsert_semantic_action(&mut self, action: SemanticAction) -> Result<(), StorageError>;
    fn upsert_semantic_action_link(&mut self, link: SemanticActionLink)
    -> Result<(), StorageError>;
    fn upsert_file_observation_paths(
        &mut self,
        paths: &[FileObservationPath],
    ) -> Result<(), StorageError>;
    fn upsert_file_path_sets(&mut self, path_sets: &[FilePathSetWrite])
    -> Result<(), StorageError>;
    fn list_semantic_actions(&self, trace_id: TraceId)
    -> Result<Vec<SemanticAction>, StorageError>;
    fn list_semantic_action_links(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticActionLink>, StorageError>;
    fn semantic_actions_matching_kinds(
        &self,
        trace_id: TraceId,
        kinds: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError>;
    fn semantic_action_summary(
        &self,
        trace_id: TraceId,
    ) -> Result<SemanticActionSummary, StorageError>;
    fn observed_agent_semantic_action(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<SemanticAction>, StorageError>;
    fn semantic_action_children(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, StorageError>;
    fn semantic_action_children_page(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionChildPage, StorageError>;
    /// Reads the web display-root page without building the full action projection.
    ///
    /// This is a storage-level display query; web-only legacy LLM normalization and
    /// global cycle removal remain owned by the full projection path.
    fn semantic_action_display_root_children_page(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
        root_link_roles: &[&str],
        page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionDisplayRootChildPage, StorageError>;
    fn semantic_action_display_root_child_count(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
    ) -> Result<usize, StorageError>;
    fn semantic_action_children_matching_kinds(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
        child_roles: &[&str],
        child_kinds: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, StorageError>;
    fn semantic_action_by_id(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Option<SemanticAction>, StorageError>;
    fn file_path_set_paths_page(
        &self,
        trace_id: TraceId,
        action_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Option<FilePathSetPathPage>, StorageError>;
    fn semantic_action_command_fallback_children(
        &self,
        trace_id: TraceId,
        command: &SemanticAction,
        display_parent_roles: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError>;
    fn semantic_action_for_process_kind(
        &self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        kind: &str,
    ) -> Result<Option<SemanticAction>, StorageError>;
    fn semantic_action_child_count(
        &self,
        trace_id: TraceId,
        parent_action_id: &str,
        roles: &[&str],
    ) -> Result<usize, StorageError>;

    fn acquire_export_lease(&mut self, trace_id: TraceId) -> Result<ExportLease, StorageError>;
    fn release_export_lease(&mut self, lease: ExportLease) -> Result<(), StorageError>;
    fn read_snapshot(&self, lease: &ExportLease) -> Result<SnapshotView, StorageError>;

    fn list_terminal_candidates(&self) -> Result<Vec<RetentionCandidate>, StorageError>;
    fn purge_trace(
        &mut self,
        trace_id: TraceId,
        tombstone: TraceTombstone,
    ) -> Result<(), StorageError>;
}

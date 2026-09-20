use std::collections::BTreeMap;
use std::time::SystemTime;

use alert_contract::{
    AlertDefinition, AlertDefinitionId, AlertDraft, AlertId, AlertListLimit, AlertStoreError,
    AlertStoreErrorKind, AlertSubmitOutcome, AlertView,
};
use model_core::diagnostics::{DiagnosticRecord, LlmPipelineDiagnostic};
use model_core::event::DomainEvent;
use model_core::external_cgroup::{ExternalBindingStaleReason, ExternalCgroupBinding};
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use model_core::process::{ProcessIdentity, ProcessMembership, ProcessRecord};
use model_core::resource_scope::{ResourceScopeLifecycleState, TraceResourceScope};
use model_core::trace::{TraceAlertToken, TraceHealth, TraceLifecycleState, TraceRecord};
use semantic_action::{
    FileObservationPath, FilePathSetPathPage, FilePathSetWrite, LlmRequestContentPage,
    LlmRequestContentWrite, LlmRequestLineage, LlmRequestLineageWrite, McpJsonRpcContentPage,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionLink, SemanticActionPage,
    SemanticActionUpdate,
};
use storage_core::{
    PayloadSegmentQuery, RetentionCandidate, SemanticActionChildPage, SemanticActionChildPageQuery,
    SemanticActionChildRow, SemanticActionDisplayPathEntry, SemanticActionDisplayRootChildPage,
    SemanticActionSummary, SemanticActionTraceRevision, SnapshotView, StorageBackend, StorageError,
    StorageTransaction, TlsFlowDiagnostic, TraceFilter, TraceLease, TraceLeasePurpose,
    TraceTombstone,
};

use crate::storage::NoOpStorage;
use crate::transaction::NoOpTransaction;

impl StorageBackend for NoOpStorage {
    fn retains_observations(&self) -> bool {
        false
    }

    fn next_trace_id_seed(&self) -> Result<u64, StorageError> {
        Ok(1)
    }

    fn next_event_id_seed(&self) -> Result<u64, StorageError> {
        Ok(1)
    }

    fn next_diagnostic_id_seed(&self) -> Result<u64, StorageError> {
        Ok(1)
    }

    fn next_payload_segment_id_seed(&self) -> Result<u64, StorageError> {
        Ok(1)
    }

    fn reserve_process_id_block(&mut self, count: u64) -> Result<(u64, u64), StorageError> {
        if count == 0 {
            return Err(StorageError::new(
                "process_id_range",
                "range must not be empty",
            ));
        }
        let start = self.next_process_id;
        let end = start.checked_add(count).ok_or_else(|| {
            StorageError::new("process_id_range", "process identity range exhausted")
        })?;
        self.next_process_id = end;
        Ok((start, end))
    }

    fn upsert_process_record(&mut self, _record: ProcessRecord) -> Result<(), StorageError> {
        Ok(())
    }

    fn get_process_record(
        &self,
        _identity: ProcessIdentity,
    ) -> Result<Option<ProcessRecord>, StorageError> {
        Ok(None)
    }

    fn list_process_records(&self) -> Result<Vec<ProcessRecord>, StorageError> {
        Ok(Vec::new())
    }

    fn begin(&mut self) -> Result<Box<dyn StorageTransaction>, StorageError> {
        Ok(Box::new(NoOpTransaction::begin(self.resources.clone())?))
    }

    fn create_trace(&mut self, trace: TraceRecord) -> Result<(), StorageError> {
        self.resources.borrow_mut().trace_created(trace.trace_id);
        Ok(())
    }

    fn update_trace_lifecycle(
        &mut self,
        _trace_id: TraceId,
        _state: TraceLifecycleState,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn update_trace_health(
        &mut self,
        _trace_id: TraceId,
        _health: TraceHealth,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn get_trace(&self, _trace_id: TraceId) -> Result<Option<TraceRecord>, StorageError> {
        Ok(None)
    }

    fn list_traces(&self, _filter: &TraceFilter) -> Result<Vec<TraceRecord>, StorageError> {
        Ok(Vec::new())
    }

    fn upsert_membership(&mut self, _membership: ProcessMembership) -> Result<(), StorageError> {
        Ok(())
    }

    fn trace_memberships(
        &self,
        _trace_id: TraceId,
    ) -> Result<Vec<ProcessMembership>, StorageError> {
        Ok(Vec::new())
    }

    fn append_event(&mut self, _event: DomainEvent) -> Result<(), StorageError> {
        Ok(())
    }

    fn create_resource_scope(&mut self, scope: TraceResourceScope) -> Result<(), StorageError> {
        self.resources.borrow_mut().create_scope(scope)
    }

    fn get_resource_scope(&self, id: TraceId) -> Result<Option<TraceResourceScope>, StorageError> {
        Ok(self.resources.borrow().scope(id))
    }

    fn list_resource_scopes(&self) -> Result<Vec<TraceResourceScope>, StorageError> {
        Ok(self.resources.borrow().scopes())
    }

    fn update_resource_scope_state(
        &mut self,
        id: TraceId,
        state: ResourceScopeLifecycleState,
        event: Option<model_core::ids::EventId>,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        self.resources
            .borrow_mut()
            .update_scope(id, state, event, time)
    }

    fn create_external_cgroup_binding(
        &mut self,
        binding: ExternalCgroupBinding,
    ) -> Result<(), StorageError> {
        self.resources.borrow_mut().create_binding(binding)
    }

    fn get_external_cgroup_binding(
        &self,
        id: TraceId,
    ) -> Result<Option<ExternalCgroupBinding>, StorageError> {
        Ok(self.resources.borrow().binding(id))
    }

    fn list_live_external_cgroup_bindings(
        &self,
    ) -> Result<Vec<ExternalCgroupBinding>, StorageError> {
        Ok(self.resources.borrow().live_bindings())
    }

    fn record_external_cgroup_success(
        &mut self,
        id: TraceId,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        self.resources.borrow_mut().success(id, time)
    }

    fn record_external_cgroup_failure(
        &mut self,
        id: TraceId,
        time: SystemTime,
    ) -> Result<u32, StorageError> {
        self.resources.borrow_mut().failure(id, time)
    }

    fn mark_external_cgroup_stale(
        &mut self,
        id: TraceId,
        reason: ExternalBindingStaleReason,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        self.resources.borrow_mut().stale(id, reason, time)
    }

    fn discard_orphan_external_binding(&mut self, id: TraceId) -> Result<(), StorageError> {
        self.resources.borrow_mut().discard_orphan(id);
        Ok(())
    }

    fn append_final_event_and_close_external_binding(
        &mut self,
        event: DomainEvent,
        time: SystemTime,
    ) -> Result<model_core::ids::EventId, StorageError> {
        self.resources
            .borrow_mut()
            .close(event.envelope.trace_id, event.envelope.event_id, time)
    }

    fn list_events(&self, _trace_id: TraceId) -> Result<Vec<DomainEvent>, StorageError> {
        Ok(Vec::new())
    }

    fn count_events_by_variant(
        &self,
        _trace_id: TraceId,
    ) -> Result<BTreeMap<String, usize>, StorageError> {
        Ok(BTreeMap::new())
    }

    fn append_payload_segment(&mut self, _segment: PayloadSegment) -> Result<(), StorageError> {
        Ok(())
    }

    fn list_payload_segments(
        &self,
        _trace_id: TraceId,
        _query: PayloadSegmentQuery,
    ) -> Result<Vec<PayloadSegment>, StorageError> {
        Ok(Vec::new())
    }

    fn count_payload_segments(&self, _trace_id: TraceId) -> Result<usize, StorageError> {
        Ok(0)
    }

    fn retained_payload_bytes(&self, _trace_id: TraceId) -> Result<u64, StorageError> {
        Ok(0)
    }

    fn append_diagnostic(&mut self, _diagnostic: DiagnosticRecord) -> Result<(), StorageError> {
        Ok(())
    }

    fn list_diagnostics(&self, _trace_id: TraceId) -> Result<Vec<DiagnosticRecord>, StorageError> {
        Ok(Vec::new())
    }

    fn append_llm_pipeline_diagnostics(
        &mut self,
        _rows: &[LlmPipelineDiagnostic],
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn list_llm_pipeline_diagnostics(
        &self,
        _trace_id: TraceId,
    ) -> Result<Vec<LlmPipelineDiagnostic>, StorageError> {
        Ok(Vec::new())
    }

    fn append_tls_flow_diagnostics(
        &mut self,
        _rows: Vec<TlsFlowDiagnostic>,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn list_tls_flow_diagnostics(
        &self,
        _trace_id: TraceId,
    ) -> Result<Vec<TlsFlowDiagnostic>, StorageError> {
        Ok(Vec::new())
    }

    fn register_alert_definition(
        &mut self,
        definition: &AlertDefinition,
    ) -> Result<AlertDefinitionId, AlertStoreError> {
        definition.validate().map_err(|message| {
            AlertStoreError::new(
                AlertStoreErrorKind::InvalidDefinition,
                "register_alert_definition",
                message,
            )
        })?;
        let id = self.next_definition_id;
        self.next_definition_id = id.checked_add(1).ok_or_else(|| {
            AlertStoreError::new(
                AlertStoreErrorKind::StorageFailure,
                "register_alert_definition",
                "definition identity range exhausted",
            )
        })?;
        // Registration IDs are opaque to the caller; no definition is retained.
        Ok(AlertDefinitionId::new(id))
    }

    fn submit_alert(
        &mut self,
        _trace_id: TraceId,
        _token: &TraceAlertToken,
        _producer: &str,
        _draft: &AlertDraft,
        _created_at: SystemTime,
    ) -> Result<AlertSubmitOutcome, AlertStoreError> {
        Ok(AlertSubmitOutcome::NotPersisted)
    }

    fn latest_alerts(&self, _limit: AlertListLimit) -> Result<Vec<AlertView>, AlertStoreError> {
        Ok(Vec::new())
    }

    fn get_alert(&self, _alert_id: AlertId) -> Result<Option<AlertView>, AlertStoreError> {
        Ok(None)
    }

    fn trace_alerts(
        &self,
        _trace_id: TraceId,
        _limit: AlertListLimit,
    ) -> Result<Vec<AlertView>, AlertStoreError> {
        Ok(Vec::new())
    }

    fn insert_semantic_action(&mut self, _action: SemanticAction) -> Result<(), StorageError> {
        Ok(())
    }

    fn update_semantic_action(
        &mut self,
        _update: SemanticActionUpdate,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn upsert_semantic_action_link(
        &mut self,
        _link: SemanticActionLink,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn upsert_file_observation_paths(
        &mut self,
        _paths: &[FileObservationPath],
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn upsert_file_path_sets(&mut self, _paths: &[FilePathSetWrite]) -> Result<(), StorageError> {
        Ok(())
    }

    fn upsert_llm_request_contents(
        &mut self,
        _contents: &[LlmRequestContentWrite],
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn upsert_llm_request_lineages(
        &mut self,
        _lineages: &[LlmRequestLineageWrite],
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn upsert_mcp_jsonrpc_contents(
        &mut self,
        _contents: &[McpJsonRpcContentWrite],
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn list_semantic_actions(
        &self,
        _trace_id: TraceId,
    ) -> Result<Vec<SemanticAction>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_actions_page(
        &self,
        _trace_id: TraceId,
        _offset: usize,
        _limit: usize,
    ) -> Result<SemanticActionPage, StorageError> {
        Ok(SemanticActionPage {
            actions: Vec::new(),
            next_offset: None,
        })
    }

    fn llm_request_lineage(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
    ) -> Result<Option<LlmRequestLineage>, StorageError> {
        Ok(None)
    }

    fn llm_request_lineages(
        &self,
        _trace_id: TraceId,
    ) -> Result<Vec<LlmRequestLineage>, StorageError> {
        Ok(Vec::new())
    }

    fn llm_request_trajectory(
        &self,
        _trace_id: TraceId,
        _trajectory_id: &str,
    ) -> Result<Vec<LlmRequestLineage>, StorageError> {
        Ok(Vec::new())
    }

    fn llm_request_forks(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
    ) -> Result<Vec<LlmRequestLineage>, StorageError> {
        Ok(Vec::new())
    }

    fn list_file_observation_paths(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
    ) -> Result<Vec<FileObservationPath>, StorageError> {
        Ok(Vec::new())
    }

    fn list_semantic_action_links(
        &self,
        _trace_id: TraceId,
    ) -> Result<Vec<SemanticActionLink>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_action_links_matching_roles(
        &self,
        _trace_id: TraceId,
        _roles: &[&str],
    ) -> Result<Vec<SemanticActionLink>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_actions_matching_kinds(
        &self,
        _trace_id: TraceId,
        _kinds: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_actions_matching_kinds_lite(
        &self,
        _trace_id: TraceId,
        _kinds: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_action_summary(
        &self,
        _trace_id: TraceId,
    ) -> Result<SemanticActionSummary, StorageError> {
        Ok(SemanticActionSummary {
            actions: 0,
            links: 0,
            roots: 0,
        })
    }

    fn semantic_action_trace_revision(
        &self,
        _trace_id: TraceId,
    ) -> Result<SemanticActionTraceRevision, StorageError> {
        Ok(SemanticActionTraceRevision {
            action_count: 0,
            action_max_key: 0,
            link_count: 0,
            link_max_rowid: 0,
            state_revision: 0,
        })
    }

    fn observed_agent_semantic_action(
        &self,
        _trace_id: TraceId,
    ) -> Result<Option<SemanticAction>, StorageError> {
        Ok(None)
    }

    fn semantic_action_children(
        &self,
        _trace_id: TraceId,
        _parent: &str,
        _roles: &[&str],
        _child_roles: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_action_children_page(
        &self,
        _trace_id: TraceId,
        _parent: &str,
        _roles: &[&str],
        _child_roles: &[&str],
        _page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionChildPage, StorageError> {
        Ok(SemanticActionChildPage {
            rows: Vec::new(),
            total_count: 0,
        })
    }

    fn semantic_action_display_root_children_page(
        &self,
        _trace_id: TraceId,
        _parent_roles: &[&str],
        _root_roles: &[&str],
        _page: SemanticActionChildPageQuery,
    ) -> Result<SemanticActionDisplayRootChildPage, StorageError> {
        Ok(SemanticActionDisplayRootChildPage {
            rows: Vec::new(),
            total_count: 0,
        })
    }

    fn semantic_action_display_root_child_count(
        &self,
        _trace_id: TraceId,
        _parent_roles: &[&str],
    ) -> Result<usize, StorageError> {
        Ok(0)
    }

    fn semantic_action_display_path_to_kind(
        &self,
        _trace_id: TraceId,
        _parent_roles: &[&str],
        _target: &str,
        _after: Option<&str>,
    ) -> Result<Option<Vec<SemanticActionDisplayPathEntry>>, StorageError> {
        Ok(None)
    }

    fn semantic_action_children_matching_kinds(
        &self,
        _trace_id: TraceId,
        _parent: &str,
        _roles: &[&str],
        _child_roles: &[&str],
        _child_kinds: &[&str],
    ) -> Result<Vec<SemanticActionChildRow>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_action_by_id(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
    ) -> Result<Option<SemanticAction>, StorageError> {
        Ok(None)
    }

    fn file_path_set_paths_page(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
        _offset: usize,
        _limit: usize,
    ) -> Result<Option<FilePathSetPathPage>, StorageError> {
        Ok(None)
    }

    fn llm_request_content_page(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
        _max_bytes: usize,
    ) -> Result<Option<LlmRequestContentPage>, StorageError> {
        Ok(None)
    }

    fn mcp_jsonrpc_content_page(
        &self,
        _trace_id: TraceId,
        _action_id: &str,
        _max_bytes: usize,
    ) -> Result<Option<McpJsonRpcContentPage>, StorageError> {
        Ok(None)
    }

    fn semantic_action_command_fallback_children(
        &self,
        _trace_id: TraceId,
        _command: &SemanticAction,
        _roles: &[&str],
    ) -> Result<Vec<SemanticAction>, StorageError> {
        Ok(Vec::new())
    }

    fn semantic_action_for_process_kind(
        &self,
        _trace_id: TraceId,
        _process: &ProcessIdentity,
        _kind: &str,
    ) -> Result<Option<SemanticAction>, StorageError> {
        Ok(None)
    }

    fn semantic_action_child_count(
        &self,
        _trace_id: TraceId,
        _parent: &str,
        _roles: &[&str],
    ) -> Result<usize, StorageError> {
        Ok(0)
    }

    fn acquire_trace_lease(
        &mut self,
        _trace_id: TraceId,
        _purpose: TraceLeasePurpose,
    ) -> Result<TraceLease, StorageError> {
        Err(Self::trace_not_found())
    }

    fn release_trace_lease(&mut self, _lease: TraceLease) -> Result<(), StorageError> {
        Ok(())
    }

    fn read_snapshot(&self, _lease: &TraceLease) -> Result<SnapshotView, StorageError> {
        Err(Self::trace_not_found())
    }

    fn list_terminal_candidates(&self) -> Result<Vec<RetentionCandidate>, StorageError> {
        Ok(Vec::new())
    }

    fn purge_trace(
        &mut self,
        _trace_id: TraceId,
        _tombstone: TraceTombstone,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<(), StorageError> {
        Ok(())
    }
}

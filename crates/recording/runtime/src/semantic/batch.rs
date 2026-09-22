use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use semantic_action::{
    FileObservationPath, FilePathSetWrite, LlmRequestContentWrite, LlmRequestLineageWrite,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionLink, SemanticActionUpdate,
};

use super::error::RecordingError;

const SEMANTIC_ACTION_BATCH_STAGE: &str = "semantic_action_batch";

#[derive(Clone, Default)]
pub struct SemanticActionBatch {
    actions: Vec<SemanticAction>,
    updates: Vec<SemanticActionUpdate>,
    updated_actions: Vec<SemanticAction>,
    links: Vec<SemanticActionLink>,
    file_observation_paths: Vec<FileObservationPath>,
    file_path_sets: Vec<FilePathSetWrite>,
    llm_request_contents: Vec<LlmRequestContentWrite>,
    llm_request_lineages: Vec<LlmRequestLineageWrite>,
    mcp_jsonrpc_contents: Vec<McpJsonRpcContentWrite>,
    payload_segments: Vec<PayloadSegment>,
}

impl SemanticActionBatch {
    pub fn from_parts(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        Self {
            actions,
            updates: Vec::new(),
            updated_actions: Vec::new(),
            links,
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            llm_request_contents: Vec::new(),
            llm_request_lineages: Vec::new(),
            mcp_jsonrpc_contents: Vec::new(),
            payload_segments: Vec::new(),
        }
    }

    pub fn from_action_output(
        actions: Vec<SemanticAction>,
        updates: Vec<SemanticActionUpdate>,
        updated_actions: Vec<SemanticAction>,
        links: Vec<SemanticActionLink>,
        file_observation_paths: Vec<FileObservationPath>,
        file_path_sets: Vec<FilePathSetWrite>,
        llm_request_contents: Vec<LlmRequestContentWrite>,
        llm_request_lineages: Vec<LlmRequestLineageWrite>,
        mcp_jsonrpc_contents: Vec<McpJsonRpcContentWrite>,
        payload_segments: Vec<PayloadSegment>,
    ) -> Self {
        Self {
            actions,
            updates,
            updated_actions,
            links,
            file_observation_paths,
            file_path_sets,
            llm_request_contents,
            llm_request_lineages,
            mcp_jsonrpc_contents,
            payload_segments,
        }
    }

    pub fn actions(&self) -> &[SemanticAction] {
        &self.actions
    }

    pub fn actions_mut(&mut self) -> &mut Vec<SemanticAction> {
        &mut self.actions
    }

    pub fn updated_actions_mut(&mut self) -> &mut Vec<SemanticAction> {
        &mut self.updated_actions
    }

    pub fn push_update(&mut self, update: SemanticActionUpdate) {
        self.updates.push(update);
    }

    pub fn action_views(&self) -> impl Iterator<Item = &SemanticAction> {
        self.actions.iter().chain(&self.updated_actions)
    }

    pub fn retain_evidence(&mut self, retain: impl Fn(&semantic_action::SemanticEvidence) -> bool) {
        for action in self.actions.iter_mut().chain(&mut self.updated_actions) {
            action.evidence.retain(&retain);
        }
        for update in &mut self.updates {
            update.evidence.retain(&retain);
        }
    }

    pub(super) fn take_persistence_updates(&mut self) -> Vec<SemanticActionUpdate> {
        std::mem::take(&mut self.updates)
    }

    pub(super) fn take_persistence_actions(&mut self) -> Vec<SemanticAction> {
        std::mem::take(&mut self.actions)
    }

    pub fn links(&self) -> &[SemanticActionLink] {
        &self.links
    }

    pub fn has_durable_records(&self) -> bool {
        self.actions
            .iter()
            .any(|action| super::SemanticActionRecorder::persists_action_kind(action.kind))
            || !self.updates.is_empty()
            || !self.links.is_empty()
            || self.has_auxiliary_records()
            || !self.payload_segments.is_empty()
    }

    pub fn has_live_export_records(&self) -> bool {
        self.action_views().any(super::export::action_exportable)
    }

    pub(super) fn take_persistence_links(&mut self) -> Vec<SemanticActionLink> {
        std::mem::take(&mut self.links)
    }

    pub(super) fn has_auxiliary_records(&self) -> bool {
        !self.file_observation_paths.is_empty()
            || !self.file_path_sets.is_empty()
            || !self.llm_request_contents.is_empty()
            || !self.llm_request_lineages.is_empty()
            || !self.mcp_jsonrpc_contents.is_empty()
    }

    pub fn file_observation_paths(&self) -> &[FileObservationPath] {
        &self.file_observation_paths
    }

    pub fn file_path_sets(&self) -> &[FilePathSetWrite] {
        &self.file_path_sets
    }

    pub fn llm_request_contents(&self) -> &[LlmRequestContentWrite] {
        &self.llm_request_contents
    }

    pub fn llm_request_lineages(&self) -> &[LlmRequestLineageWrite] {
        &self.llm_request_lineages
    }

    pub fn push_llm_request_lineage(&mut self, lineage: LlmRequestLineageWrite) {
        self.llm_request_lineages.push(lineage);
    }

    pub fn extend_llm_request_lineages(
        &mut self,
        lineages: impl IntoIterator<Item = LlmRequestLineageWrite>,
    ) {
        self.llm_request_lineages.extend(lineages);
    }

    pub fn mcp_jsonrpc_contents(&self) -> &[McpJsonRpcContentWrite] {
        &self.mcp_jsonrpc_contents
    }

    pub fn payload_segments(&self) -> &[PayloadSegment] {
        &self.payload_segments
    }

    pub(super) fn take_payload_segments(&mut self) -> Vec<PayloadSegment> {
        std::mem::take(&mut self.payload_segments)
    }

    pub fn as_record_batch(&self) -> SemanticActionRecordBatch<'_> {
        SemanticActionRecordBatch::new(
            &self.actions,
            &self.links,
            &self.file_observation_paths,
            &self.file_path_sets,
            &self.llm_request_contents,
            &self.llm_request_lineages,
            &self.mcp_jsonrpc_contents,
        )
        .with_updates(&self.updates, &self.updated_actions)
        .with_payload_segments(&self.payload_segments)
    }

    pub fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.updates.extend(other.updates);
        self.updated_actions.extend(other.updated_actions);
        self.links.extend(other.links);
        self.file_observation_paths
            .extend(other.file_observation_paths);
        self.file_path_sets.extend(other.file_path_sets);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.llm_request_lineages.extend(other.llm_request_lineages);
        self.mcp_jsonrpc_contents.extend(other.mcp_jsonrpc_contents);
        self.payload_segments.extend(other.payload_segments);
    }

    pub(crate) fn split_by_trace(self) -> Vec<Self> {
        let mut batches = BTreeMap::<TraceId, Self>::new();
        for update in self.updates {
            batches
                .entry(update.trace_id)
                .or_default()
                .updates
                .push(update);
        }
        for action in self.updated_actions {
            batches
                .entry(action.trace_id)
                .or_default()
                .updated_actions
                .push(action);
        }
        for action in self.actions {
            batches
                .entry(action.trace_id)
                .or_default()
                .actions
                .push(action);
        }
        for link in self.links {
            batches.entry(link.trace_id).or_default().links.push(link);
        }
        for path in self.file_observation_paths {
            batches
                .entry(path.trace_id)
                .or_default()
                .file_observation_paths
                .push(path);
        }
        for path_set in self.file_path_sets {
            batches
                .entry(path_set.trace_id)
                .or_default()
                .file_path_sets
                .push(path_set);
        }
        for content in self.llm_request_contents {
            batches
                .entry(content.manifest.trace_id)
                .or_default()
                .llm_request_contents
                .push(content);
        }
        for lineage in self.llm_request_lineages {
            batches
                .entry(lineage.trace_id)
                .or_default()
                .llm_request_lineages
                .push(lineage);
        }
        for content in self.mcp_jsonrpc_contents {
            batches
                .entry(content.trace_id)
                .or_default()
                .mcp_jsonrpc_contents
                .push(content);
        }
        for segment in self.payload_segments {
            batches
                .entry(segment.trace_id)
                .or_default()
                .payload_segments
                .push(segment);
        }
        batches.into_values().collect()
    }

    pub fn into_parts(self) -> (Vec<SemanticAction>, Vec<SemanticActionLink>) {
        (self.actions, self.links)
    }
}

pub struct SemanticActionRecordBatch<'a> {
    actions: &'a [SemanticAction],
    updates: &'a [SemanticActionUpdate],
    updated_actions: &'a [SemanticAction],
    links: &'a [SemanticActionLink],
    file_observation_paths: &'a [FileObservationPath],
    file_path_sets: &'a [FilePathSetWrite],
    llm_request_contents: &'a [LlmRequestContentWrite],
    llm_request_lineages: &'a [LlmRequestLineageWrite],
    mcp_jsonrpc_contents: &'a [McpJsonRpcContentWrite],
    payload_segments: &'a [PayloadSegment],
}

impl<'a> SemanticActionRecordBatch<'a> {
    pub fn new(
        actions: &'a [SemanticAction],
        links: &'a [SemanticActionLink],
        file_observation_paths: &'a [FileObservationPath],
        file_path_sets: &'a [FilePathSetWrite],
        llm_request_contents: &'a [LlmRequestContentWrite],
        llm_request_lineages: &'a [LlmRequestLineageWrite],
        mcp_jsonrpc_contents: &'a [McpJsonRpcContentWrite],
    ) -> Self {
        Self {
            actions,
            updates: &[],
            updated_actions: &[],
            links,
            file_observation_paths,
            file_path_sets,
            llm_request_contents,
            llm_request_lineages,
            mcp_jsonrpc_contents,
            payload_segments: &[],
        }
    }

    pub fn actions(&self) -> &'a [SemanticAction] {
        self.actions
    }

    pub(crate) fn with_payload_segments(mut self, segments: &'a [PayloadSegment]) -> Self {
        self.payload_segments = segments;
        self
    }

    pub(crate) fn payload_segments(&self) -> &'a [PayloadSegment] {
        self.payload_segments
    }

    pub fn with_updates(
        mut self,
        updates: &'a [SemanticActionUpdate],
        updated_actions: &'a [SemanticAction],
    ) -> Self {
        self.updates = updates;
        self.updated_actions = updated_actions;
        self
    }

    pub fn updates(&self) -> &'a [SemanticActionUpdate] {
        self.updates
    }

    pub fn action_views(&self) -> impl Iterator<Item = &'a SemanticAction> {
        self.actions.iter().chain(self.updated_actions)
    }

    pub fn links(&self) -> &'a [SemanticActionLink] {
        self.links
    }

    pub fn file_observation_paths(&self) -> &'a [FileObservationPath] {
        self.file_observation_paths
    }

    pub fn file_path_sets(&self) -> &'a [FilePathSetWrite] {
        self.file_path_sets
    }

    pub fn llm_request_contents(&self) -> &'a [LlmRequestContentWrite] {
        self.llm_request_contents
    }

    pub fn llm_request_lineages(&self) -> &'a [LlmRequestLineageWrite] {
        self.llm_request_lineages
    }

    pub fn mcp_jsonrpc_contents(&self) -> &'a [McpJsonRpcContentWrite] {
        self.mcp_jsonrpc_contents
    }

    pub fn trace_id(&self) -> Result<Option<TraceId>, RecordingError> {
        let mut trace_id = None;
        for update in self.updates {
            record_trace_id(&mut trace_id, update.trace_id)?;
        }
        for action in self.updated_actions {
            record_trace_id(&mut trace_id, action.trace_id)?;
        }
        for action in self.actions {
            record_trace_id(&mut trace_id, action.trace_id)?;
        }
        for link in self.links {
            record_trace_id(&mut trace_id, link.trace_id)?;
        }
        for path in self.file_observation_paths {
            record_trace_id(&mut trace_id, path.trace_id)?;
        }
        for path_set in self.file_path_sets {
            record_trace_id(&mut trace_id, path_set.trace_id)?;
        }
        for content in self.llm_request_contents {
            record_trace_id(&mut trace_id, content.manifest.trace_id)?;
        }
        for lineage in self.llm_request_lineages {
            record_trace_id(&mut trace_id, lineage.trace_id)?;
        }
        for content in self.mcp_jsonrpc_contents {
            record_trace_id(&mut trace_id, content.trace_id)?;
        }
        Ok(trace_id)
    }
}

fn record_trace_id(current: &mut Option<TraceId>, trace_id: TraceId) -> Result<(), RecordingError> {
    match current {
        Some(existing) if *existing != trace_id => Err(RecordingError::new(
            SEMANTIC_ACTION_BATCH_STAGE,
            "semantic action batch spans multiple traces",
        )),
        Some(_) => Ok(()),
        None => {
            *current = Some(trace_id);
            Ok(())
        }
    }
}

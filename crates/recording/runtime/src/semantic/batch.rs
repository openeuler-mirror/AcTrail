use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use semantic_action::{
    FileObservationPath, FilePathSetWrite, LlmRequestContentWrite, McpJsonRpcContentWrite,
    SemanticAction, SemanticActionLink,
};

use super::error::RecordingError;

const SEMANTIC_ACTION_BATCH_STAGE: &str = "semantic_action_batch";

#[derive(Clone, Default)]
pub struct SemanticActionBatch {
    actions: Vec<SemanticAction>,
    links: Vec<SemanticActionLink>,
    file_observation_paths: Vec<FileObservationPath>,
    file_path_sets: Vec<FilePathSetWrite>,
    llm_request_contents: Vec<LlmRequestContentWrite>,
    mcp_jsonrpc_contents: Vec<McpJsonRpcContentWrite>,
    payload_segments: Vec<PayloadSegment>,
}

impl SemanticActionBatch {
    pub fn from_parts(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        Self {
            actions,
            links,
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            llm_request_contents: Vec::new(),
            mcp_jsonrpc_contents: Vec::new(),
            payload_segments: Vec::new(),
        }
    }

    pub fn from_action_output(
        actions: Vec<SemanticAction>,
        links: Vec<SemanticActionLink>,
        file_observation_paths: Vec<FileObservationPath>,
        file_path_sets: Vec<FilePathSetWrite>,
        llm_request_contents: Vec<LlmRequestContentWrite>,
        mcp_jsonrpc_contents: Vec<McpJsonRpcContentWrite>,
        payload_segments: Vec<PayloadSegment>,
    ) -> Self {
        Self {
            actions,
            links,
            file_observation_paths,
            file_path_sets,
            llm_request_contents,
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

    pub(super) fn take_persistence_actions(&mut self) -> Vec<SemanticAction> {
        std::mem::take(&mut self.actions)
    }

    pub fn links(&self) -> &[SemanticActionLink] {
        &self.links
    }

    pub(super) fn take_persistence_links(&mut self) -> Vec<SemanticActionLink> {
        std::mem::take(&mut self.links)
    }

    pub(super) fn has_auxiliary_records(&self) -> bool {
        !self.file_observation_paths.is_empty()
            || !self.file_path_sets.is_empty()
            || !self.llm_request_contents.is_empty()
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
            &self.mcp_jsonrpc_contents,
        )
    }

    pub fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
        self.file_observation_paths
            .extend(other.file_observation_paths);
        self.file_path_sets.extend(other.file_path_sets);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.mcp_jsonrpc_contents.extend(other.mcp_jsonrpc_contents);
        self.payload_segments.extend(other.payload_segments);
    }

    pub(crate) fn split_by_trace(self) -> Vec<Self> {
        let mut batches = BTreeMap::<TraceId, Self>::new();
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
        for content in self.mcp_jsonrpc_contents {
            batches
                .entry(content.trace_id)
                .or_default()
                .mcp_jsonrpc_contents
                .push(content);
        }
        batches.into_values().collect()
    }

    pub fn into_parts(self) -> (Vec<SemanticAction>, Vec<SemanticActionLink>) {
        (self.actions, self.links)
    }
}

pub struct SemanticActionRecordBatch<'a> {
    actions: &'a [SemanticAction],
    links: &'a [SemanticActionLink],
    file_observation_paths: &'a [FileObservationPath],
    file_path_sets: &'a [FilePathSetWrite],
    llm_request_contents: &'a [LlmRequestContentWrite],
    mcp_jsonrpc_contents: &'a [McpJsonRpcContentWrite],
}

impl<'a> SemanticActionRecordBatch<'a> {
    pub fn new(
        actions: &'a [SemanticAction],
        links: &'a [SemanticActionLink],
        file_observation_paths: &'a [FileObservationPath],
        file_path_sets: &'a [FilePathSetWrite],
        llm_request_contents: &'a [LlmRequestContentWrite],
        mcp_jsonrpc_contents: &'a [McpJsonRpcContentWrite],
    ) -> Self {
        Self {
            actions,
            links,
            file_observation_paths,
            file_path_sets,
            llm_request_contents,
            mcp_jsonrpc_contents,
        }
    }

    pub fn actions(&self) -> &'a [SemanticAction] {
        self.actions
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

    pub fn mcp_jsonrpc_contents(&self) -> &'a [McpJsonRpcContentWrite] {
        self.mcp_jsonrpc_contents
    }

    pub fn trace_id(&self) -> Result<Option<TraceId>, RecordingError> {
        let mut trace_id = None;
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

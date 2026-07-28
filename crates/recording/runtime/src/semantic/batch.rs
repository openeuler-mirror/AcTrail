use std::collections::BTreeMap;

use model_core::ids::TraceId;
use semantic_action::{
    FileObservationPath, FilePathSetWrite, LlmRequestContentWrite, SemanticAction,
    SemanticActionLink,
};

use super::error::RecordingError;

const SEMANTIC_ACTION_BATCH_STAGE: &str = "semantic_action_batch";

#[derive(Default)]
pub struct SemanticActionBatch {
    actions: Vec<SemanticAction>,
    links: Vec<SemanticActionLink>,
    file_observation_paths: Vec<FileObservationPath>,
    file_path_sets: Vec<FilePathSetWrite>,
    llm_request_contents: Vec<LlmRequestContentWrite>,
}

impl SemanticActionBatch {
    pub fn from_parts(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        Self {
            actions,
            links,
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            llm_request_contents: Vec::new(),
        }
    }

    pub fn from_action_output(
        actions: Vec<SemanticAction>,
        links: Vec<SemanticActionLink>,
        file_observation_paths: Vec<FileObservationPath>,
        file_path_sets: Vec<FilePathSetWrite>,
        llm_request_contents: Vec<LlmRequestContentWrite>,
    ) -> Self {
        Self {
            actions,
            links,
            file_observation_paths,
            file_path_sets,
            llm_request_contents,
        }
    }

    pub fn actions(&self) -> &[SemanticAction] {
        &self.actions
    }

    pub fn links(&self) -> &[SemanticActionLink] {
        &self.links
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

    pub fn as_record_batch(&self) -> SemanticActionRecordBatch<'_> {
        SemanticActionRecordBatch::new(
            &self.actions,
            &self.links,
            &self.file_observation_paths,
            &self.file_path_sets,
            &self.llm_request_contents,
        )
    }

    pub fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
        self.file_observation_paths
            .extend(other.file_observation_paths);
        self.file_path_sets.extend(other.file_path_sets);
        self.llm_request_contents.extend(other.llm_request_contents);
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
}

impl<'a> SemanticActionRecordBatch<'a> {
    pub fn new(
        actions: &'a [SemanticAction],
        links: &'a [SemanticActionLink],
        file_observation_paths: &'a [FileObservationPath],
        file_path_sets: &'a [FilePathSetWrite],
        llm_request_contents: &'a [LlmRequestContentWrite],
    ) -> Self {
        Self {
            actions,
            links,
            file_observation_paths,
            file_path_sets,
            llm_request_contents,
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

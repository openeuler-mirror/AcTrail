use model_core::ids::TraceId;
use semantic_action::{FileObservationPath, FilePathSetWrite, SemanticAction, SemanticActionLink};

use super::error::RecordingError;

const SEMANTIC_ACTION_BATCH_STAGE: &str = "semantic_action_batch";

#[derive(Default)]
pub struct SemanticActionBatch {
    actions: Vec<SemanticAction>,
    links: Vec<SemanticActionLink>,
    file_observation_paths: Vec<FileObservationPath>,
    file_path_sets: Vec<FilePathSetWrite>,
}

impl SemanticActionBatch {
    pub fn from_parts(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        Self {
            actions,
            links,
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
        }
    }

    pub fn from_action_output(
        actions: Vec<SemanticAction>,
        links: Vec<SemanticActionLink>,
        file_observation_paths: Vec<FileObservationPath>,
        file_path_sets: Vec<FilePathSetWrite>,
    ) -> Self {
        Self {
            actions,
            links,
            file_observation_paths,
            file_path_sets,
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

    pub fn as_record_batch(&self) -> SemanticActionRecordBatch<'_> {
        SemanticActionRecordBatch::new(
            &self.actions,
            &self.links,
            &self.file_observation_paths,
            &self.file_path_sets,
        )
    }

    pub fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
        self.file_observation_paths
            .extend(other.file_observation_paths);
        self.file_path_sets.extend(other.file_path_sets);
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
}

impl<'a> SemanticActionRecordBatch<'a> {
    pub fn new(
        actions: &'a [SemanticAction],
        links: &'a [SemanticActionLink],
        file_observation_paths: &'a [FileObservationPath],
        file_path_sets: &'a [FilePathSetWrite],
    ) -> Self {
        Self {
            actions,
            links,
            file_observation_paths,
            file_path_sets,
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

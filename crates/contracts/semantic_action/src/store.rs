//! Storage boundary for semantic actions.

use model_core::ids::TraceId;

use crate::model::{
    FileObservationPath, FilePathSetPathPage, FilePathSetWrite, SemanticAction, SemanticActionLink,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionStoreError {
    pub stage: String,
    pub message: String,
}

impl SemanticActionStoreError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

pub trait SemanticActionWriteStore {
    fn upsert_semantic_action(
        &mut self,
        action: SemanticAction,
    ) -> Result<(), SemanticActionStoreError>;

    fn upsert_semantic_action_link(
        &mut self,
        link: SemanticActionLink,
    ) -> Result<(), SemanticActionStoreError>;

    fn upsert_file_observation_paths(
        &mut self,
        paths: &[FileObservationPath],
    ) -> Result<(), SemanticActionStoreError>;

    fn upsert_file_path_sets(
        &mut self,
        path_sets: &[FilePathSetWrite],
    ) -> Result<(), SemanticActionStoreError>;
}

pub trait SemanticActionReadStore {
    fn list_semantic_actions(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticAction>, SemanticActionStoreError>;

    fn list_semantic_action_links(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticActionLink>, SemanticActionStoreError>;

    fn file_path_set_paths_page(
        &self,
        trace_id: TraceId,
        action_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Option<FilePathSetPathPage>, SemanticActionStoreError>;
}

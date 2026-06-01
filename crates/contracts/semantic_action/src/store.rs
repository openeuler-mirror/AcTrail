//! Storage boundary for semantic actions.

use model_core::ids::TraceId;

use crate::model::SemanticAction;

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
}

pub trait SemanticActionReadStore {
    fn list_semantic_actions(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticAction>, SemanticActionStoreError>;
}

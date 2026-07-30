use semantic_action::SemanticActionKind;
use storage_core::StorageBackend;

use super::{RecordingError, SemanticActionRecordBatch};

pub(crate) struct SemanticActionRecorder<'a> {
    storage: &'a mut dyn StorageBackend,
}

impl<'a> SemanticActionRecorder<'a> {
    pub(crate) fn new(storage: &'a mut dyn StorageBackend) -> Self {
        Self { storage }
    }

    pub(crate) fn persist_batch(
        &mut self,
        batch: SemanticActionRecordBatch<'_>,
    ) -> Result<(), RecordingError> {
        // Persist actions before links so graph edges never race ahead of their nodes.
        for action in batch
            .actions()
            .iter()
            .filter(|action| Self::persists_action_kind(action.kind))
            .cloned()
        {
            self.storage.upsert_semantic_action(action)?;
        }
        for link in batch.links().iter().cloned() {
            self.storage.upsert_semantic_action_link(link)?;
        }
        self.storage
            .upsert_file_observation_paths(batch.file_observation_paths())?;
        self.storage.upsert_file_path_sets(batch.file_path_sets())?;
        self.storage
            .upsert_llm_request_contents(batch.llm_request_contents())?;
        Ok(())
    }

    fn persists_action_kind(kind: SemanticActionKind) -> bool {
        // Termination is durable in process records and raw exit events. The
        // semantic exit actions exist only to make the online export boundary explicit.
        !matches!(
            kind,
            SemanticActionKind::ProcessExit | SemanticActionKind::AgentExit
        )
    }
}

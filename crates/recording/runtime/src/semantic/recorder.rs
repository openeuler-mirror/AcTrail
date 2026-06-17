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
        for action in batch.actions().iter().cloned() {
            self.storage.upsert_semantic_action(action)?;
        }
        for link in batch.links().iter().cloned() {
            self.storage.upsert_semantic_action_link(link)?;
        }
        self.storage
            .upsert_file_observation_paths(batch.file_observation_paths())?;
        self.storage.upsert_file_path_sets(batch.file_path_sets())?;
        Ok(())
    }
}

use std::sync::atomic::{AtomicU64, Ordering};

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionUpdate,
};
use storage_core::StorageBackend;

use super::{RecordingError, SemanticActionRecordBatch};

static LINEAGE_PERSISTENCE_FAILURES: AtomicU64 = AtomicU64::new(0);

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
        self.persist_graph(
            batch
                .actions()
                .iter()
                .filter(|action| Self::persists_action_kind(action.kind))
                .cloned(),
            batch.updates().iter().cloned(),
            batch.links().iter().cloned(),
        )?;
        self.storage
            .upsert_file_observation_paths(batch.file_observation_paths())?;
        self.storage.upsert_file_path_sets(batch.file_path_sets())?;
        self.storage
            .upsert_llm_request_contents(batch.llm_request_contents())?;
        if let Err(error) = self
            .storage
            .upsert_llm_request_lineages(batch.llm_request_lineages())
        {
            let failure_count = LINEAGE_PERSISTENCE_FAILURES
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            if failure_count.is_power_of_two() {
                eprintln!(
                    "warning: LLM request lineage persistence failed locally: failures={} stage={} message={}",
                    failure_count, error.stage, error.message
                );
            }
        }
        self.storage
            .upsert_mcp_jsonrpc_contents(batch.mcp_jsonrpc_contents())?;
        Ok(())
    }

    pub(super) fn persist_graph(
        &mut self,
        actions: impl IntoIterator<Item = SemanticAction>,
        updates: impl IntoIterator<Item = SemanticActionUpdate>,
        links: impl IntoIterator<Item = SemanticActionLink>,
    ) -> Result<(), RecordingError> {
        // Persist actions before links so graph edges never race ahead of their nodes.
        for action in actions {
            self.storage.insert_semantic_action(action)?;
        }
        for update in updates {
            self.storage.update_semantic_action(update)?;
        }
        for link in links {
            self.storage.upsert_semantic_action_link(link)?;
        }
        Ok(())
    }

    pub(super) fn persists_action_kind(kind: SemanticActionKind) -> bool {
        // Termination is durable in process records and raw exit events. The
        // semantic exit actions exist only to make the online export boundary explicit.
        !matches!(
            kind,
            SemanticActionKind::ProcessExit | SemanticActionKind::AgentExit
        )
    }
}

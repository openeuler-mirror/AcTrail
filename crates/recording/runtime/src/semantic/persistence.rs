//! Transaction-local compaction of repeated semantic persistence updates.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use semantic_action::{SemanticAction, SemanticActionLink};
use storage_core::StorageBackend;

use super::{RecordingError, SemanticActionBatch, SemanticActionRecorder};

#[derive(Default)]
pub(crate) struct SemanticActionPersistenceAccumulator {
    actions: Vec<SemanticAction>,
    action_indexes: HashMap<TraceId, HashMap<String, usize>>,
    links: Vec<SemanticActionLink>,
    link_indexes: HashMap<u64, Vec<usize>>,
    auxiliary_batches: Vec<SemanticActionBatch>,
    payload_segments: Vec<PayloadSegment>,
}

impl SemanticActionPersistenceAccumulator {
    pub(crate) fn push_batch(
        &mut self,
        mut batch: SemanticActionBatch,
    ) -> Result<(), RecordingError> {
        for action in batch
            .take_persistence_actions()
            .into_iter()
            .filter(|action| SemanticActionRecorder::persists_action_kind(action.kind))
        {
            self.push_action(action)?;
        }
        for link in batch.take_persistence_links() {
            self.push_link(link);
        }
        self.payload_segments.extend(batch.take_payload_segments());
        if batch.has_auxiliary_records() {
            self.auxiliary_batches.push(batch);
        }
        Ok(())
    }

    pub(crate) fn persist(self, storage: &mut dyn StorageBackend) -> Result<(), RecordingError> {
        let Self {
            actions,
            links,
            auxiliary_batches,
            payload_segments,
            ..
        } = self;
        let mut recorder = SemanticActionRecorder::new(storage);
        if !actions.is_empty() || !links.is_empty() {
            let graph = SemanticActionBatch::from_parts(actions, links);
            recorder.persist_batch(graph.as_record_batch())?;
        }
        for auxiliary in auxiliary_batches {
            recorder.persist_batch(auxiliary.as_record_batch())?;
        }
        for segment in payload_segments {
            storage.append_payload_segment(segment)?;
        }
        Ok(())
    }

    fn push_action(&mut self, action: SemanticAction) -> Result<(), RecordingError> {
        let indexes = self.action_indexes.entry(action.trace_id).or_default();
        if let Some(index) = indexes.get(action.action_id.as_str()).copied() {
            self.actions[index]
                .merge_persistence_update(action)
                .map_err(|error| {
                    RecordingError::new("merge_semantic_action", error.into_message())
                })?;
            return Ok(());
        }

        let index = self.actions.len();
        indexes.insert(action.action_id.clone(), index);
        self.actions.push(action);
        Ok(())
    }

    fn push_link(&mut self, link: SemanticActionLink) {
        let identity_hash = Self::link_identity_hash(&link);
        let existing_index = self.link_indexes.get(&identity_hash).and_then(|indexes| {
            indexes
                .iter()
                .copied()
                .find(|index| Self::links_have_same_identity(&self.links[*index], &link))
        });
        if let Some(index) = existing_index {
            self.links[index] = link;
            return;
        }

        let index = self.links.len();
        self.link_indexes
            .entry(identity_hash)
            .or_default()
            .push(index);
        self.links.push(link);
    }

    fn link_identity_hash(link: &SemanticActionLink) -> u64 {
        let mut hasher = DefaultHasher::new();
        link.trace_id.hash(&mut hasher);
        link.parent_action_id.hash(&mut hasher);
        link.child_action_id.hash(&mut hasher);
        link.role.as_str().hash(&mut hasher);
        hasher.finish()
    }

    fn links_have_same_identity(left: &SemanticActionLink, right: &SemanticActionLink) -> bool {
        left.trace_id == right.trace_id
            && left.parent_action_id == right.parent_action_id
            && left.child_action_id == right.child_action_id
            && left.role == right.role
    }
}

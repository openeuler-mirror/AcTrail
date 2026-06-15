use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionLink};

use super::error::RecordingError;

const SEMANTIC_ACTION_BATCH_STAGE: &str = "semantic_action_batch";

#[derive(Default)]
pub struct SemanticActionBatch {
    actions: Vec<SemanticAction>,
    links: Vec<SemanticActionLink>,
}

impl SemanticActionBatch {
    pub fn from_parts(actions: Vec<SemanticAction>, links: Vec<SemanticActionLink>) -> Self {
        Self { actions, links }
    }

    pub fn actions(&self) -> &[SemanticAction] {
        &self.actions
    }

    pub fn links(&self) -> &[SemanticActionLink] {
        &self.links
    }

    pub fn as_record_batch(&self) -> SemanticActionRecordBatch<'_> {
        SemanticActionRecordBatch::new(&self.actions, &self.links)
    }

    pub fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
    }

    pub fn into_parts(self) -> (Vec<SemanticAction>, Vec<SemanticActionLink>) {
        (self.actions, self.links)
    }
}

pub struct SemanticActionRecordBatch<'a> {
    actions: &'a [SemanticAction],
    links: &'a [SemanticActionLink],
}

impl<'a> SemanticActionRecordBatch<'a> {
    pub fn new(actions: &'a [SemanticAction], links: &'a [SemanticActionLink]) -> Self {
        Self { actions, links }
    }

    pub fn actions(&self) -> &'a [SemanticAction] {
        self.actions
    }

    pub fn links(&self) -> &'a [SemanticActionLink] {
        self.links
    }

    pub fn trace_id(&self) -> Result<Option<TraceId>, RecordingError> {
        let mut trace_id = None;
        for action in self.actions {
            record_trace_id(&mut trace_id, action.trace_id)?;
        }
        for link in self.links {
            record_trace_id(&mut trace_id, link.trace_id)?;
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

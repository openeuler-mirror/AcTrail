//! Domain merge semantics for repeated semantic action persistence updates.

use std::fmt;
use std::time::SystemTime;

use crate::model::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, SemanticEvidence,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionMergeError {
    message: String,
}

impl SemanticActionMergeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn into_message(self) -> String {
        self.message
    }
}

impl fmt::Display for SemanticActionMergeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SemanticActionMergeError {}

impl SemanticAction {
    pub fn merge_persistence_update(
        &mut self,
        mut incoming: Self,
    ) -> Result<(), SemanticActionMergeError> {
        self.validate_persistence_update(&incoming)?;
        incoming.start_time = self.start_time.min(incoming.start_time);
        incoming.end_time = merge_end_time(self.end_time, incoming.end_time);
        incoming.status = merge_status(self.status, incoming.status);
        incoming.completeness = merge_completeness(self.completeness, incoming.completeness);
        incoming.confidence_millis = incoming.confidence_millis.or(self.confidence_millis);

        let mut attributes = std::mem::take(&mut self.attributes);
        attributes.extend(incoming.attributes);
        incoming.attributes = attributes;

        incoming.evidence = merge_evidence(std::mem::take(&mut self.evidence), incoming.evidence);
        *self = incoming;
        Ok(())
    }

    fn validate_persistence_update(&self, incoming: &Self) -> Result<(), SemanticActionMergeError> {
        if self.action_id != incoming.action_id
            || self.trace_id != incoming.trace_id
            || self.kind != incoming.kind
            || self.process != incoming.process
        {
            return Err(SemanticActionMergeError::new(format!(
                "semantic action id collision for {}: existing kind={} trace={} process={}, incoming kind={} trace={} process={}",
                incoming.action_id,
                self.kind.as_str(),
                self.trace_id,
                self.process,
                incoming.kind.as_str(),
                incoming.trace_id,
                incoming.process,
            )));
        }
        Ok(())
    }
}

fn merge_end_time(
    existing: Option<SystemTime>,
    incoming: Option<SystemTime>,
) -> Option<SystemTime> {
    match (existing, incoming) {
        (Some(existing), Some(incoming)) => Some(existing.max(incoming)),
        (Some(existing), None) => Some(existing),
        (None, incoming) => incoming,
    }
}

fn merge_status(
    existing: SemanticActionStatus,
    incoming: SemanticActionStatus,
) -> SemanticActionStatus {
    match (existing, incoming) {
        (SemanticActionStatus::Error, _) | (_, SemanticActionStatus::Error) => {
            SemanticActionStatus::Error
        }
        (SemanticActionStatus::Success, SemanticActionStatus::InProgress)
        | (SemanticActionStatus::Success, SemanticActionStatus::Unknown)
        | (SemanticActionStatus::InProgress, SemanticActionStatus::Success)
        | (SemanticActionStatus::Unknown, SemanticActionStatus::Success)
        | (SemanticActionStatus::Success, SemanticActionStatus::Success) => {
            SemanticActionStatus::Success
        }
        (SemanticActionStatus::Unknown, SemanticActionStatus::InProgress)
        | (SemanticActionStatus::InProgress, SemanticActionStatus::Unknown)
        | (SemanticActionStatus::Unknown, SemanticActionStatus::Unknown) => {
            SemanticActionStatus::Unknown
        }
        (SemanticActionStatus::InProgress, SemanticActionStatus::InProgress) => {
            SemanticActionStatus::InProgress
        }
    }
}

fn merge_completeness(
    existing: SemanticActionCompleteness,
    incoming: SemanticActionCompleteness,
) -> SemanticActionCompleteness {
    match (existing, incoming) {
        (SemanticActionCompleteness::Complete, _) | (_, SemanticActionCompleteness::Complete) => {
            SemanticActionCompleteness::Complete
        }
        (SemanticActionCompleteness::Partial, _) | (_, SemanticActionCompleteness::Partial) => {
            SemanticActionCompleteness::Partial
        }
        (SemanticActionCompleteness::Inferred, SemanticActionCompleteness::Inferred) => {
            SemanticActionCompleteness::Inferred
        }
    }
}

fn merge_evidence(
    mut existing: Vec<SemanticEvidence>,
    incoming: Vec<SemanticEvidence>,
) -> Vec<SemanticEvidence> {
    for evidence in incoming {
        if !existing.contains(&evidence) {
            existing.push(evidence);
        }
    }
    existing
}

//! Monotonic merge rules for repeated semantic action upserts.

use std::time::SystemTime;

use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, SemanticActionStoreError,
    SemanticEvidence,
};

pub(super) fn merge_action(
    existing: SemanticAction,
    mut incoming: SemanticAction,
) -> Result<SemanticAction, SemanticActionStoreError> {
    validate_action_merge(&existing, &incoming)?;
    incoming.start_time = existing.start_time.min(incoming.start_time);
    incoming.end_time = merge_end_time(existing.end_time, incoming.end_time);
    incoming.status = merge_status(existing.status, incoming.status);
    incoming.completeness = merge_completeness(existing.completeness, incoming.completeness);
    incoming.confidence_millis = incoming.confidence_millis.or(existing.confidence_millis);

    let mut attributes = existing.attributes;
    attributes.extend(incoming.attributes);
    incoming.attributes = attributes;

    incoming.evidence = merge_evidence(existing.evidence, incoming.evidence);
    Ok(incoming)
}

fn validate_action_merge(
    existing: &SemanticAction,
    incoming: &SemanticAction,
) -> Result<(), SemanticActionStoreError> {
    if existing.trace_id != incoming.trace_id
        || existing.kind != incoming.kind
        || existing.process != incoming.process
    {
        return Err(SemanticActionStoreError::new(
            "merge_semantic_action",
            format!(
                "semantic action id collision for {}: existing kind={} trace={} process={}, incoming kind={} trace={} process={}",
                incoming.action_id,
                existing.kind.as_str(),
                existing.trace_id,
                existing.process,
                incoming.kind.as_str(),
                incoming.trace_id,
                incoming.process,
            ),
        ));
    }
    Ok(())
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

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::{NamespaceIdentity, ProcessIdentity};
use semantic_action::{SemanticAction, SemanticEvidence};

use super::actions::{
    ATTR_PROCESS_PARENT_GENERATION, ATTR_PROCESS_PARENT_IDENTITY_STATE, ATTR_PROCESS_PARENT_PID,
    ATTR_PROCESS_PARENT_PID_NAMESPACE, ATTR_PROCESS_PARENT_START_TIME_TICKS,
    ATTR_PROCESS_PARENT_TASK_ID, PROCESS_PARENT_IDENTITY_STATE_CONFLICT,
    PROCESS_PARENT_IDENTITY_STATE_OBSERVED, append_missing_evidence, event_evidence,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ForkProcessEdge {
    pub(super) trace_id: TraceId,
    pub(super) child: ProcessIdentity,
    pub(super) parent: Option<ProcessIdentity>,
    pub(super) observed_at: SystemTime,
    pub(super) evidence: Vec<SemanticEvidence>,
    pub(super) conflict: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParentEdgeApply {
    Changed,
    Unchanged,
}

pub(super) fn fork_edge_from_event(event: &DomainEvent) -> Option<ForkProcessEdge> {
    let EventPayload::Process(payload) = &event.payload else {
        return None;
    };
    if payload.operation != "fork" {
        return None;
    }
    Some(ForkProcessEdge {
        trace_id: event.envelope.trace_id,
        child: event.envelope.process.clone(),
        parent: Some(payload.parent.clone()?),
        observed_at: event.envelope.observed_at,
        evidence: vec![event_evidence(event, "process.fork")],
        conflict: false,
    })
}

pub(super) fn merge_fork_edges(
    existing: Option<&ForkProcessEdge>,
    incoming: ForkProcessEdge,
) -> ForkProcessEdge {
    let Some(existing) = existing else {
        return incoming;
    };
    let mut evidence = existing.evidence.clone();
    append_missing_evidence(&mut evidence, &incoming.evidence);
    if existing.conflict || incoming.conflict || existing.parent != incoming.parent {
        return ForkProcessEdge {
            trace_id: existing.trace_id,
            child: existing.child.clone(),
            parent: None,
            observed_at: existing.observed_at.min(incoming.observed_at),
            evidence,
            conflict: true,
        };
    }
    ForkProcessEdge {
        trace_id: existing.trace_id,
        child: existing.child.clone(),
        parent: existing.parent.clone(),
        observed_at: existing.observed_at.min(incoming.observed_at),
        evidence,
        conflict: false,
    }
}

pub(super) fn apply_fork_parent(
    action: &mut SemanticAction,
    edge: &ForkProcessEdge,
) -> ParentEdgeApply {
    if edge.conflict {
        return mark_parent_identity_conflict(action, &edge.evidence);
    }
    if parent_identity_has_conflict(action) {
        return ParentEdgeApply::Unchanged;
    }
    let Some(parent) = &edge.parent else {
        return ParentEdgeApply::Unchanged;
    };
    if parent_process_from_action(action)
        .as_ref()
        .is_some_and(|existing| existing != parent)
    {
        return mark_parent_identity_conflict(action, &edge.evidence);
    }
    let mut changed = false;
    changed |= insert_missing_attr(
        &mut action.attributes,
        ATTR_PROCESS_PARENT_PID,
        parent.pid.to_string(),
    );
    if let Some(task_id) = parent.task_id {
        changed |= insert_missing_attr(
            &mut action.attributes,
            ATTR_PROCESS_PARENT_TASK_ID,
            task_id.to_string(),
        );
    }
    changed |= insert_missing_attr(
        &mut action.attributes,
        ATTR_PROCESS_PARENT_START_TIME_TICKS,
        parent.start_time_ticks.to_string(),
    );
    if let Some(pid_namespace) = &parent.pid_namespace {
        changed |= insert_missing_attr(
            &mut action.attributes,
            ATTR_PROCESS_PARENT_PID_NAMESPACE,
            pid_namespace.as_str().to_string(),
        );
    }
    changed |= insert_missing_attr(
        &mut action.attributes,
        ATTR_PROCESS_PARENT_GENERATION,
        parent.generation.to_string(),
    );
    changed |= upsert_attr_if_changed(
        &mut action.attributes,
        ATTR_PROCESS_PARENT_IDENTITY_STATE,
        PROCESS_PARENT_IDENTITY_STATE_OBSERVED.to_string(),
    );
    if changed {
        append_missing_evidence(&mut action.evidence, &edge.evidence);
        ParentEdgeApply::Changed
    } else {
        ParentEdgeApply::Unchanged
    }
}

pub(super) fn parent_identity_has_conflict(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(ATTR_PROCESS_PARENT_IDENTITY_STATE)
        .is_some_and(|state| state == PROCESS_PARENT_IDENTITY_STATE_CONFLICT)
}

pub(super) fn parent_process_from_action(action: &SemanticAction) -> Option<ProcessIdentity> {
    if !parent_identity_is_observed(action) {
        return None;
    }
    let pid = parse_u32_attr(action, ATTR_PROCESS_PARENT_PID)?;
    let start_time_ticks = parse_u64_attr(action, ATTR_PROCESS_PARENT_START_TIME_TICKS)?;
    let generation = parse_u64_attr(action, ATTR_PROCESS_PARENT_GENERATION)?;
    let mut process = ProcessIdentity::new(pid, start_time_ticks, generation);
    if let Some(task_id) = parse_u32_attr(action, ATTR_PROCESS_PARENT_TASK_ID) {
        process = process.with_task_id(task_id);
    }
    if let Some(pid_namespace) = action.attributes.get(ATTR_PROCESS_PARENT_PID_NAMESPACE) {
        process = process.with_namespace(NamespaceIdentity::new(pid_namespace.clone()));
    }
    Some(process)
}

pub(super) fn is_parent_identity_attr(key: &str) -> bool {
    parent_identity_attrs().contains(&key)
}

fn parent_identity_is_observed(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(ATTR_PROCESS_PARENT_IDENTITY_STATE)
        .is_some_and(|state| state == PROCESS_PARENT_IDENTITY_STATE_OBSERVED)
}

fn mark_parent_identity_conflict(
    action: &mut SemanticAction,
    evidence: &[SemanticEvidence],
) -> ParentEdgeApply {
    let mut changed = !parent_identity_has_conflict(action);
    for key in parent_identity_attrs() {
        changed |= action.attributes.remove(key).is_some();
    }
    changed |= upsert_attr_if_changed(
        &mut action.attributes,
        ATTR_PROCESS_PARENT_IDENTITY_STATE,
        PROCESS_PARENT_IDENTITY_STATE_CONFLICT.to_string(),
    );
    let before = action.evidence.len();
    append_missing_evidence(&mut action.evidence, evidence);
    changed |= action.evidence.len() != before;
    if changed {
        ParentEdgeApply::Changed
    } else {
        ParentEdgeApply::Unchanged
    }
}

fn insert_missing_attr(
    attributes: &mut BTreeMap<String, String>,
    key: &str,
    value: String,
) -> bool {
    if attributes.contains_key(key) {
        return false;
    }
    attributes.insert(key.to_string(), value);
    true
}

fn upsert_attr_if_changed(
    attributes: &mut BTreeMap<String, String>,
    key: &str,
    value: String,
) -> bool {
    if attributes
        .get(key)
        .is_some_and(|existing| existing == &value)
    {
        return false;
    }
    attributes.insert(key.to_string(), value);
    true
}

fn parent_identity_attrs() -> [&'static str; 5] {
    [
        ATTR_PROCESS_PARENT_PID,
        ATTR_PROCESS_PARENT_TASK_ID,
        ATTR_PROCESS_PARENT_START_TIME_TICKS,
        ATTR_PROCESS_PARENT_PID_NAMESPACE,
        ATTR_PROCESS_PARENT_GENERATION,
    ]
}

fn parse_u32_attr(action: &SemanticAction, key: &str) -> Option<u32> {
    action
        .attributes
        .get(key)
        .and_then(|value| value.parse().ok())
}

fn parse_u64_attr(action: &SemanticAction, key: &str) -> Option<u64> {
    action
        .attributes
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
}

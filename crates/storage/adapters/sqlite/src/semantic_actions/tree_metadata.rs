//! Batch metadata loaders for semantic action tree children.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};
use semantic_action::{
    SemanticActionLinkRole, SemanticActionStoreError, SemanticEvidence, attr_keys as attrs,
};

use crate::semantic_actions::codebook::sqlite::{
    LinkEvidenceKey, decode_link_role, link_role_code, link_role_code_from_str,
};
use crate::semantic_actions::store::evidence_from_row;
use crate::semantic_actions::tree::SemanticActionChildRow;

pub(super) fn load_child_metadata(
    connection: &Connection,
    trace_id: TraceId,
    children: &mut [SemanticActionChildRow],
    child_roles: &[&str],
) -> Result<(), SemanticActionStoreError> {
    let action_ids = children
        .iter()
        .map(|child| child.action.action_id.clone())
        .collect::<Vec<_>>();
    let mut action_evidence = read_evidence_for_actions(connection, &action_ids)?;
    let mut link_evidence = read_link_evidence_for_links(connection, children)?;
    let mut child_counts = display_child_counts(connection, trace_id, &action_ids, child_roles)?;

    for child in children {
        child.action.evidence = action_evidence
            .remove(&child.action.action_id)
            .unwrap_or_default();
        child.link.evidence = link_evidence
            .remove(&LinkEvidenceKey::from_link(&child.link))
            .unwrap_or_default();
        child.child_count = child_counts
            .remove(&child.action.action_id)
            .unwrap_or_default();
    }
    Ok(())
}

pub(super) fn child_count_for_parent(
    connection: &Connection,
    trace_id: TraceId,
    parent_action_id: &str,
    roles: &[&str],
) -> Result<usize, SemanticActionStoreError> {
    let mut counts =
        display_child_counts(connection, trace_id, &[parent_action_id.to_string()], roles)?;
    Ok(counts.remove(parent_action_id).unwrap_or_default())
}

pub(super) fn invalidated_action_attrs(
    attributes: &std::collections::BTreeMap<String, String>,
) -> bool {
    attributes
        .get(attrs::actrail::ACTION_VALID)
        .is_some_and(|value| value == "false")
}

pub(super) fn invalidated_link_attrs(
    link_attrs: &std::collections::BTreeMap<String, String>,
    role: SemanticActionLinkRole,
    action_attrs: &std::collections::BTreeMap<String, String>,
) -> bool {
    link_attrs
        .get(attrs::actrail::LINK_VALID)
        .is_some_and(|value| value == "false")
        || ((role == SemanticActionLinkRole::AgentPerformedAction
            || role == SemanticActionLinkRole::CommandContainsCommandInvocation
            || role == SemanticActionLinkRole::CommandContainsMcpToolCall)
            && action_attrs
                .get(attrs::process_parent::IDENTITY_STATE)
                .is_some_and(|value| value == "conflict"))
}

pub(super) fn effective_incoming_link_absence_predicate(child_alias: &str) -> String {
    effective_link_absence_predicate(child_alias, None)
}

pub(super) fn display_parent_link_absence_predicate(roles: &[&str], child_alias: &str) -> String {
    if roles.is_empty() {
        return String::new();
    }
    effective_link_absence_predicate(child_alias, Some(roles))
}

pub(super) fn push_display_parent_link_values(
    values: &mut Vec<Value>,
    roles: &[&str],
) -> Result<(), SemanticActionStoreError> {
    if roles.is_empty() {
        return Ok(());
    }
    push_effective_link_values(values, Some(roles))
}

pub(super) fn push_effective_link_values(
    values: &mut Vec<Value>,
    roles: Option<&[&str]>,
) -> Result<(), SemanticActionStoreError> {
    if let Some(roles) = roles {
        for role in roles {
            values.push(Value::Integer(i64::from(link_role_code_from_str(role)?)));
        }
    }
    values.push(Value::Integer(i64::from(link_role_code(
        SemanticActionLinkRole::AgentPerformedAction,
    ))));
    values.push(Value::Integer(i64::from(link_role_code(
        SemanticActionLinkRole::CommandContainsCommandInvocation,
    ))));
    values.push(Value::Integer(i64::from(link_role_code(
        SemanticActionLinkRole::CommandContainsMcpToolCall,
    ))));
    Ok(())
}

pub(super) fn display_parent_link_value_count(roles: &[&str]) -> usize {
    if roles.is_empty() {
        0
    } else {
        effective_link_value_count(Some(roles))
    }
}

pub(super) fn effective_link_value_count(roles: Option<&[&str]>) -> usize {
    roles.map_or(3, |roles| roles.len() + 3)
}

fn read_evidence_for_actions(
    connection: &Connection,
    action_ids: &[String],
) -> Result<BTreeMap<String, Vec<SemanticEvidence>>, SemanticActionStoreError> {
    if action_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let query = format!(
        "SELECT ids.action_id, evidence.kind_code, evidence.evidence_id, evidence.role
         FROM semantic_action_evidence evidence
         JOIN semantic_action_ids ids
           ON ids.action_key = evidence.action_key
         WHERE ids.action_id IN ({})
         ORDER BY ids.action_id ASC, evidence.evidence_order ASC",
        sql_placeholders(action_ids.len())
    );
    let values = action_ids
        .iter()
        .map(|action_id| Value::Text(action_id.clone()))
        .collect::<Vec<_>>();
    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new("prepare_semantic_action_evidence_batch", error.to_string())
    })?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>("action_id")?, evidence_from_row(row)?))
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_evidence_batch", error.to_string())
        })?;
    let mut evidence = BTreeMap::new();
    for row in rows {
        let (action_id, item) = row.map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_evidence_batch", error.to_string())
        })?;
        evidence
            .entry(action_id)
            .or_insert_with(Vec::new)
            .push(item);
    }
    Ok(evidence)
}

fn read_link_evidence_for_links(
    connection: &Connection,
    children: &[SemanticActionChildRow],
) -> Result<BTreeMap<LinkEvidenceKey, Vec<SemanticEvidence>>, SemanticActionStoreError> {
    if children.is_empty() {
        return Ok(BTreeMap::new());
    }
    let requested = children
        .iter()
        .map(|child| LinkEvidenceKey::from_link(&child.link))
        .collect::<BTreeSet<_>>();
    let trace_id = children[0].link.trace_id;
    let parent_ids = children
        .iter()
        .map(|child| child.link.parent_action_id.clone())
        .collect::<BTreeSet<_>>();
    let child_ids = children
        .iter()
        .map(|child| child.link.child_action_id.clone())
        .collect::<BTreeSet<_>>();
    let roles = children
        .iter()
        .map(|child| link_role_code(child.link.role))
        .collect::<BTreeSet<_>>();
    let query = format!(
        "SELECT evidence.trace_id,
                parent_ids.action_id AS parent_action_id,
                child_ids.action_id AS child_action_id,
                evidence.role_code AS link_role_code,
                evidence.kind_code, evidence.evidence_id, evidence.evidence_role
         FROM semantic_action_link_evidence evidence
         JOIN semantic_action_ids parent_ids
           ON parent_ids.action_key = evidence.parent_action_key
         JOIN semantic_action_ids child_ids
           ON child_ids.action_key = evidence.child_action_key
         WHERE evidence.trace_id = ?
           AND parent_ids.action_id IN ({})
           AND child_ids.action_id IN ({})
           AND evidence.role_code IN ({})
         ORDER BY parent_ids.action_id ASC, child_ids.action_id ASC, link_role_code ASC, evidence.evidence_order ASC",
        sql_placeholders(parent_ids.len()),
        sql_placeholders(child_ids.len()),
        sql_placeholders(roles.len())
    );
    let values = link_evidence_query_values(trace_id, &parent_ids, &child_ids, &roles)?;
    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new(
            "prepare_semantic_action_link_evidence_batch",
            error.to_string(),
        )
    })?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((LinkEvidenceKey::from_row(row)?, evidence_from_row(row)?))
        })
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_link_evidence_batch",
                error.to_string(),
            )
        })?;
    let mut evidence = BTreeMap::new();
    for row in rows {
        let (key, item) = row.map_err(|error| {
            SemanticActionStoreError::new(
                "map_semantic_action_link_evidence_batch",
                error.to_string(),
            )
        })?;
        if requested.contains(&key) {
            evidence.entry(key).or_insert_with(Vec::new).push(item);
        }
    }
    Ok(evidence)
}

pub(super) fn display_child_counts(
    connection: &Connection,
    trace_id: TraceId,
    parent_action_ids: &[String],
    roles: &[&str],
) -> Result<BTreeMap<String, usize>, SemanticActionStoreError> {
    if parent_action_ids.is_empty() || roles.is_empty() {
        return Ok(BTreeMap::new());
    }
    let parent_action_ids = parent_action_ids.iter().cloned().collect::<BTreeSet<_>>();
    let query = format!(
        "SELECT parent_ids.action_id AS parent_action_id,
                child_ids.action_id AS child_action_id,
                link.role_code,
                link.link_valid_code,
                child.action_valid_code,
                child.process_parent_conflict
         FROM semantic_action_links link
         JOIN semantic_actions child
           ON child.action_key = link.child_action_key
         JOIN semantic_action_ids parent_ids
           ON parent_ids.action_key = link.parent_action_key
         JOIN semantic_action_ids child_ids
           ON child_ids.action_key = link.child_action_key
         WHERE link.trace_id = ?
           AND parent_ids.action_id IN ({})
           AND link.role_code IN ({})",
        sql_placeholders(parent_action_ids.len()),
        sql_placeholders(roles.len())
    );
    let values = child_count_query_values(trace_id, &parent_action_ids, roles)?;
    let mut statement = connection.prepare(&query).map_err(|error| {
        SemanticActionStoreError::new("prepare_semantic_action_child_count", error.to_string())
    })?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>("parent_action_id")?,
                row.get::<_, String>("child_action_id")?,
                decode_link_role(row.get::<_, i64>("role_code")?)?,
                row.get::<_, i64>("link_valid_code")?,
                row.get::<_, i64>("action_valid_code")?,
                row.get::<_, i64>("process_parent_conflict")?,
            ))
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_child_count", error.to_string())
        })?;
    let mut children_by_parent = BTreeMap::<String, BTreeSet<String>>::new();
    for row in rows {
        let (
            parent_action_id,
            child_action_id,
            role,
            link_valid_code,
            action_valid_code,
            process_parent_conflict,
        ) = row.map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_child_count", error.to_string())
        })?;
        let role_conflicted = (role == SemanticActionLinkRole::AgentPerformedAction
            || role == SemanticActionLinkRole::CommandContainsCommandInvocation
            || role == SemanticActionLinkRole::CommandContainsMcpToolCall)
            && process_parent_conflict == 1;
        if action_valid_code == 1 && link_valid_code == 1 && !role_conflicted {
            children_by_parent
                .entry(parent_action_id)
                .or_default()
                .insert(child_action_id);
        }
    }
    Ok(children_by_parent
        .into_iter()
        .map(|(parent_action_id, children)| (parent_action_id, children.len()))
        .collect())
}

fn effective_link_absence_predicate(child_alias: &str, roles: Option<&[&str]>) -> String {
    let role_filter = roles
        .map(|roles| format!("AND link.role_code IN ({})", sql_placeholders(roles.len())))
        .unwrap_or_default();
    format!(
        "AND NOT EXISTS (
           SELECT 1
           FROM semantic_action_links link
           JOIN semantic_actions parent
             ON parent.action_key = link.parent_action_key
           WHERE link.trace_id = {child_alias}.trace_id
             AND link.child_action_key = {child_alias}.action_key
             {role_filter}
             AND link.link_valid_code = 1
             AND parent.action_valid_code = 1
             AND NOT (
               (link.role_code = ? OR link.role_code = ? OR link.role_code = ?)
               AND {child_alias}.process_parent_conflict = 1
             )
         )"
    )
}

fn child_count_query_values(
    trace_id: TraceId,
    parent_action_ids: &BTreeSet<String>,
    roles: &[&str],
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let trace_id = i64::try_from(trace_id.get()).map_err(|error| {
        SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
    })?;
    let mut values = Vec::with_capacity(1 + parent_action_ids.len() + roles.len());
    values.push(Value::Integer(trace_id));
    values.extend(parent_action_ids.iter().cloned().map(Value::Text));
    for role in roles {
        values.push(Value::Integer(i64::from(link_role_code_from_str(role)?)));
    }
    Ok(values)
}

fn link_evidence_query_values(
    trace_id: TraceId,
    parent_ids: &BTreeSet<String>,
    child_ids: &BTreeSet<String>,
    roles: &BTreeSet<i16>,
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let trace_id = i64::try_from(trace_id.get()).map_err(|error| {
        SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
    })?;
    let mut values = Vec::with_capacity(1 + parent_ids.len() + child_ids.len() + roles.len());
    values.push(Value::Integer(trace_id));
    values.extend(parent_ids.iter().cloned().map(Value::Text));
    values.extend(child_ids.iter().cloned().map(Value::Text));
    values.extend(roles.iter().map(|role| Value::Integer(i64::from(*role))));
    Ok(values)
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

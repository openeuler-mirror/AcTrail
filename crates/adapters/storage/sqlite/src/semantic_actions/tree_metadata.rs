//! Batch metadata loaders for semantic action tree children.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params_from_iter};
use semantic_action::{SemanticActionLink, SemanticActionStoreError, SemanticEvidence};

use crate::records::decode_map;
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
    let mut child_counts = action_child_counts(connection, trace_id, &action_ids, child_roles)?;

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
        action_child_counts(connection, trace_id, &[parent_action_id.to_string()], roles)?;
    Ok(counts.remove(parent_action_id).unwrap_or_default())
}

pub(super) fn invalidated_action_attrs(
    attributes: &std::collections::BTreeMap<String, String>,
) -> bool {
    attributes
        .get("actrail.action.valid")
        .is_some_and(|value| value == "false")
}

pub(super) fn invalidated_link_attrs(
    link_attrs: &std::collections::BTreeMap<String, String>,
    role: &str,
    action_attrs: &std::collections::BTreeMap<String, String>,
) -> bool {
    link_attrs
        .get("actrail.link.valid")
        .is_some_and(|value| value == "false")
        || ((role == "agent.performed_action" || role == "command.contains_command_invocation")
            && action_attrs
                .get("process.parent.identity_state")
                .is_some_and(|value| value == "conflict"))
}

fn read_evidence_for_actions(
    connection: &Connection,
    action_ids: &[String],
) -> Result<BTreeMap<String, Vec<SemanticEvidence>>, SemanticActionStoreError> {
    if action_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let query = format!(
        "SELECT action_id, kind, evidence_id, role FROM semantic_action_evidence
         WHERE action_id IN ({})
         ORDER BY action_id ASC, evidence_order ASC",
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
        .map(|child| child.link.role.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let query = format!(
        "SELECT trace_id, parent_action_id, child_action_id, role AS link_role,
                kind, evidence_id, evidence_role
         FROM semantic_action_link_evidence
         WHERE trace_id = ?
           AND parent_action_id IN ({})
           AND child_action_id IN ({})
           AND role IN ({})
         ORDER BY parent_action_id ASC, child_action_id ASC, link_role ASC, evidence_order ASC",
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

fn action_child_counts(
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
        "SELECT link.parent_action_id,
                link.child_action_id,
                link.role,
                link.attributes AS link_attributes,
                child.attributes AS child_attributes
         FROM semantic_action_links link
         JOIN semantic_actions child
           ON child.trace_id = link.trace_id
          AND child.action_id = link.child_action_id
         WHERE link.trace_id = ?
           AND link.parent_action_id IN ({})
           AND link.role IN ({})",
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
                row.get::<_, String>("role")?,
                decode_map(&row.get::<_, String>("link_attributes")?),
                decode_map(&row.get::<_, String>("child_attributes")?),
            ))
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_child_count", error.to_string())
        })?;
    let mut children_by_parent = BTreeMap::<String, BTreeSet<String>>::new();
    for row in rows {
        let (parent_action_id, child_action_id, role, link_attrs, action_attrs) =
            row.map_err(|error| {
                SemanticActionStoreError::new("map_semantic_action_child_count", error.to_string())
            })?;
        if !invalidated_action_attrs(&action_attrs)
            && !invalidated_link_attrs(&link_attrs, &role, &action_attrs)
        {
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
    values.extend(roles.iter().map(|role| Value::Text((*role).to_string())));
    Ok(values)
}

fn link_evidence_query_values(
    trace_id: TraceId,
    parent_ids: &BTreeSet<String>,
    child_ids: &BTreeSet<String>,
    roles: &BTreeSet<String>,
) -> Result<Vec<Value>, SemanticActionStoreError> {
    let trace_id = i64::try_from(trace_id.get()).map_err(|error| {
        SemanticActionStoreError::new("semantic_action_trace_id_param", error.to_string())
    })?;
    let mut values = Vec::with_capacity(1 + parent_ids.len() + child_ids.len() + roles.len());
    values.push(Value::Integer(trace_id));
    values.extend(parent_ids.iter().cloned().map(Value::Text));
    values.extend(child_ids.iter().cloned().map(Value::Text));
    values.extend(roles.iter().cloned().map(Value::Text));
    Ok(values)
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LinkEvidenceKey {
    trace_id: u64,
    parent_action_id: String,
    child_action_id: String,
    role: String,
}

impl LinkEvidenceKey {
    fn from_link(link: &SemanticActionLink) -> Self {
        Self {
            trace_id: link.trace_id.get(),
            parent_action_id: link.parent_action_id.clone(),
            child_action_id: link.child_action_id.clone(),
            role: link.role.as_str().to_string(),
        }
    }

    fn from_row(row: &Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            trace_id: row.get("trace_id")?,
            parent_action_id: row.get("parent_action_id")?,
            child_action_id: row.get("child_action_id")?,
            role: row.get("link_role")?,
        })
    }
}

use rusqlite::params;
use semantic_action::{
    SemanticAction, SemanticActionLink, SemanticActionStoreError, attr_keys as attrs,
};

use crate::records::{encode_map, encode_time};
use crate::semantic_actions::codebook::sqlite::{
    action_completeness_code, action_kind_code, action_status_code, evidence_kind_code,
};
use crate::semantic_actions::cold_fields::upsert_action_attributes;
use crate::semantic_actions::storage_meta::ColdFieldCompression;

pub(super) fn write_action_row(
    connection: &mut rusqlite::Connection,
    action_key: i64,
    action: &SemanticAction,
    compression: ColdFieldCompression,
) -> Result<(), SemanticActionStoreError> {
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO semantic_actions (
                action_key, trace_id, kind_code, title, start_time, end_time, process_id,
                status_code, completeness_code, confidence_millis,
                action_valid_code, agent_observed, process_parent_conflict, attributes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .and_then(|mut statement| {
            statement.execute(params![
                action_key,
                action.trace_id.get(),
                action_kind_code(action.kind),
                &action.title,
                encode_time(action.start_time),
                action.end_time.map(encode_time),
                action.process.get(),
                action_status_code(action.status),
                action_completeness_code(action.completeness),
                action.confidence_millis,
                action_valid_code(action),
                agent_observed(action),
                process_parent_conflict(action),
                "",
            ])
        })
        .map_err(|error| {
            SemanticActionStoreError::new("upsert_semantic_action", error.to_string())
        })?;
    upsert_action_attributes(
        connection,
        action_key,
        &encode_map(&action.attributes),
        compression,
    )
    .map_err(|error| {
        SemanticActionStoreError::new("upsert_semantic_action_attributes", error.to_string())
    })
}

pub(super) fn replace_action_evidence(
    connection: &mut rusqlite::Connection,
    action: &SemanticAction,
    known_action_key: Option<i64>,
) -> Result<(), SemanticActionStoreError> {
    let action_key = match known_action_key {
        Some(action_key) => action_key,
        None => connection
            .prepare_cached("SELECT action_key FROM semantic_action_ids WHERE action_id = ?1")
            .and_then(|mut statement| {
                statement.query_row(params![action.action_id], |row| {
                    row.get::<_, i64>("action_key")
                })
            })
            .map_err(|error| {
                SemanticActionStoreError::new("resolve_semantic_action_key", error.to_string())
            })?,
    };
    connection
        .prepare_cached("DELETE FROM semantic_action_evidence WHERE action_key = ?1")
        .and_then(|mut statement| statement.execute(params![action_key]))
        .map_err(|error| {
            SemanticActionStoreError::new("replace_semantic_action_evidence", error.to_string())
        })?;
    for (index, evidence) in action.evidence.iter().enumerate() {
        connection
            .prepare_cached(
                "INSERT INTO semantic_action_evidence (
                    action_key, evidence_order, kind_code, evidence_id, role
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    action_key,
                    index,
                    evidence_kind_code(evidence.kind),
                    evidence.id,
                    &evidence.role,
                ])
            })
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "insert_semantic_action_evidence",
                    format!(
                        "action_id={} action_kind={} evidence_kind={} evidence_id={}: {error}",
                        action.action_id,
                        action.kind.as_str(),
                        evidence.kind.as_str(),
                        evidence.id
                    ),
                )
            })?;
    }
    Ok(())
}

pub(super) fn action_row_matches(left: &SemanticAction, right: &SemanticAction) -> bool {
    left.action_id == right.action_id
        && left.trace_id == right.trace_id
        && left.kind == right.kind
        && left.title == right.title
        && left.start_time == right.start_time
        && left.end_time == right.end_time
        && left.process == right.process
        && left.status == right.status
        && left.completeness == right.completeness
        && left.confidence_millis == right.confidence_millis
        && left.attributes == right.attributes
}

pub(super) fn link_valid_code(link: &SemanticActionLink) -> i16 {
    if link.valid
        && !link
            .attributes
            .get(attrs::actrail::LINK_VALID)
            .is_some_and(|value| value == "false")
    {
        1
    } else {
        0
    }
}

fn action_valid_code(action: &SemanticAction) -> i16 {
    if action
        .attributes
        .get(attrs::actrail::ACTION_VALID)
        .is_some_and(|value| value == "false")
    {
        0
    } else {
        1
    }
}

fn agent_observed(action: &SemanticAction) -> i16 {
    if action
        .attributes
        .get(attrs::agent::IDENTITY_STATUS)
        .is_some_and(|value| value == "observed")
    {
        1
    } else {
        0
    }
}

fn process_parent_conflict(action: &SemanticAction) -> i16 {
    if action
        .attributes
        .get(attrs::process_parent::IDENTITY_STATE)
        .is_some_and(|value| value == "conflict")
    {
        1
    } else {
        0
    }
}

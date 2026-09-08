use rusqlite::params;
use semantic_action::{SemanticAction, SemanticActionStoreError, attr_keys as attrs};

use crate::records::encode_time;
use crate::semantic_actions::codebook::sqlite::{
    action_completeness_code, action_kind_code, action_status_code,
};
use crate::semantic_actions::cold_fields::upsert_action_attributes;
use crate::semantic_actions::evidence;
use crate::semantic_actions::storage_meta::ColdFieldCompression;

pub(super) fn write_action_row(
    connection: &mut rusqlite::Connection,
    action_key: i64,
    action: &SemanticAction,
    file_path_id: Option<u64>,
    stored_title: Option<&str>,
    compression: ColdFieldCompression,
) -> Result<(), SemanticActionStoreError> {
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO semantic_actions (
                action_key, trace_id, kind_code, title, file_path_id, start_time, end_time, process_id,
                status_code, completeness_code,
                action_valid_code, process_parent_conflict, evidence_blob
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )
        .and_then(|mut statement| {
            statement.execute(params![
                action_key,
                action.trace_id.get(),
                action_kind_code(action.kind),
                stored_title,
                file_path_id,
                encode_time(action.start_time),
                action.end_time.map(encode_time),
                action.process.get(),
                action_status_code(action.status),
                action_completeness_code(action.completeness),
                action_valid_code(action),
                process_parent_conflict(action),
                evidence::encode(&action.evidence)?,
            ])
        })
        .map_err(|error| {
            SemanticActionStoreError::new("upsert_semantic_action", error.to_string())
        })?;
    upsert_action_attributes(connection, action_key, &action.attributes, compression).map_err(
        |error| {
            SemanticActionStoreError::new("upsert_semantic_action_attributes", error.to_string())
        },
    )
}

pub(super) fn update_action_evidence(
    connection: &mut rusqlite::Connection,
    action_key: i64,
    action: &SemanticAction,
) -> Result<(), SemanticActionStoreError> {
    let evidence_blob = evidence::encode(&action.evidence).map_err(|error| {
        SemanticActionStoreError::new("encode_semantic_action_evidence", error.to_string())
    })?;
    connection
        .prepare_cached("UPDATE semantic_actions SET evidence_blob = ?2 WHERE action_key = ?1")
        .and_then(|mut statement| statement.execute(params![action_key, evidence_blob]))
        .and_then(|changed| {
            (changed == 1)
                .then_some(())
                .ok_or(rusqlite::Error::InvalidQuery)
        })
        .map_err(|error| {
            SemanticActionStoreError::new("update_semantic_action_evidence", error.to_string())
        })
}

pub(super) fn write_agent_identity(
    connection: &mut rusqlite::Connection,
    trace_id: u64,
    process_id: u64,
    identity_action_key: i64,
) -> Result<(), SemanticActionStoreError> {
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO agent_identities (
                trace_id, process_id, identity_action_key
            ) VALUES (?1, ?2, ?3)",
        )
        .and_then(|mut statement| {
            statement.execute(params![trace_id, process_id, identity_action_key])
        })
        .map_err(|error| {
            SemanticActionStoreError::new("write_agent_identity", error.to_string())
        })?;
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
        && left.attributes == right.attributes
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

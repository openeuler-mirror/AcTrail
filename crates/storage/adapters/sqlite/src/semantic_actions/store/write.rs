use rusqlite::params;
use semantic_action::{SemanticAction, SemanticActionStoreError, attr_keys as attrs};

use crate::records::encode_time;
use crate::semantic_actions::codebook::sqlite::action_kind_code;
use crate::semantic_actions::cold_fields::{ColdFieldEncoder, upsert_action_attributes};

pub(super) fn write_action_row(
    connection: &mut rusqlite::Connection,
    action_key: i64,
    action: &SemanticAction,
    file_path_id: Option<u64>,
    stored_title: Option<&str>,
    encoder: &ColdFieldEncoder,
) -> Result<bool, SemanticActionStoreError> {
    let inserted = connection
        .prepare_cached(
            "INSERT INTO semantic_actions (
                action_key, trace_id, kind_code, title, file_path_id, start_time, process_id,
                action_valid_code, process_parent_conflict
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(action_key) DO NOTHING",
        )
        .and_then(|mut statement| {
            statement.execute(params![
                action_key,
                action.trace_id.get(),
                action_kind_code(action.kind),
                stored_title,
                file_path_id,
                encode_time(action.start_time),
                action.process.get(),
                action_valid_code(action),
                process_parent_conflict(action),
            ])
        })
        .map_err(|error| {
            SemanticActionStoreError::new("upsert_semantic_action", error.to_string())
        })?;
    if inserted == 0 {
        return Ok(false);
    }
    upsert_action_attributes(connection, action_key, &action.attributes, encoder).map_err(
        |error| {
            SemanticActionStoreError::new("upsert_semantic_action_attributes", error.to_string())
        },
    )?;
    Ok(true)
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

use std::sync::OnceLock;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use rusqlite::{OptionalExtension, Row, params};
use semantic_action::{
    SemanticAction, SemanticActionLink, SemanticActionStoreError, attr_keys as attrs,
};

use super::{ACTION_SELECT_COLUMNS, action_cold_field_join};

use crate::records::decode_time;
use crate::semantic_actions::codebook::sqlite::{
    decode_completeness, decode_kind, decode_link_origin, decode_link_role, decode_status,
};
use crate::semantic_actions::cold_fields::decode_attributes_from_row;
use crate::semantic_actions::evidence;

/// Shared-borrow variant for query/export paths that already hold `&Connection`.
pub(in crate::semantic_actions) fn read_action_by_id_shared(
    connection: &rusqlite::Connection,
    action_id: &str,
) -> Result<Option<SemanticAction>, SemanticActionStoreError> {
    connection
        .query_row(read_action_by_id_sql(), params![action_id], action_from_row)
        .optional()
        .map_err(|error| SemanticActionStoreError::new("read_semantic_action", error.to_string()))
        .and_then(|mut action| {
            super::ActionReadHydrator::hydrate(connection, action.iter_mut(), true)?;
            Ok(action)
        })
}

fn read_action_by_id_sql() -> &'static str {
    static SQL: OnceLock<String> = OnceLock::new();
    SQL.get_or_init(|| {
        format!(
            "SELECT {ACTION_SELECT_COLUMNS}
             FROM semantic_actions action
             JOIN semantic_action_ids ids
               ON ids.action_key = action.action_key
             {}
             WHERE ids.action_id = ?1",
            action_cold_field_join()
        )
    })
    .as_str()
}

pub(in crate::semantic_actions) fn action_from_row(
    row: &Row<'_>,
) -> Result<SemanticAction, rusqlite::Error> {
    let mut attributes = decode_attributes_from_row(row)?;
    super::state::ActionStateWriter::hydrate_attributes(row, &mut attributes)?;
    let file_path_id = row.get::<_, Option<u64>>("file_path_id")?;
    let file_path = row.get::<_, Option<String>>("file_path_text")?;
    if file_path_id.is_some() && file_path.is_none() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if let Some(path) = file_path.as_ref() {
        attributes.insert(attrs::file::PATH.to_string(), path.clone());
    }
    let title = row
        .get::<_, Option<String>>("stored_title")?
        .or_else(|| file_path.clone())
        .ok_or(rusqlite::Error::InvalidQuery)?;
    Ok(SemanticAction {
        action_id: row.get("action_id")?,
        trace_id: TraceId::new(row.get("trace_id")?),
        kind: decode_kind(row.get::<_, i64>("kind_code")?)?,
        title,
        start_time: decode_time(row.get("start_time")?),
        end_time: row.get::<_, Option<i64>>("end_time")?.map(decode_time),
        process: ProcessIdentity::new(row.get("process_id")?),
        status: decode_status(row.get::<_, i64>("status_code")?)?,
        completeness: decode_completeness(row.get::<_, i64>("completeness_code")?)?,
        attributes,
        evidence: Vec::new(),
    })
}

pub(in crate::semantic_actions) fn action_link_from_row(
    row: &Row<'_>,
) -> Result<SemanticActionLink, rusqlite::Error> {
    let attributes = decode_attributes_from_row(row)?;
    Ok(SemanticActionLink {
        trace_id: TraceId::new(row.get("trace_id")?),
        parent_action_id: row.get("parent_action_id")?,
        child_action_id: row.get("child_action_id")?,
        role: decode_link_role(row.get::<_, i64>("role_code")?)?,
        origin: decode_link_origin(row.get::<_, i64>("origin_code")?)?,
        valid: row.get("valid")?,
        evidence: evidence::decode(&row.get::<_, Vec<u8>>("evidence_blob")?)?,
        attributes,
    })
}

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use rusqlite::Row;
use semantic_action::{SemanticAction, SemanticActionLink, attr_keys as attrs};

use crate::records::decode_time;
use crate::semantic_actions::codebook::sqlite::{
    decode_completeness, decode_kind, decode_link_origin, decode_link_role, decode_status,
};
use crate::semantic_actions::cold_fields::decode_attributes_from_row;
use crate::semantic_actions::evidence;

pub(in crate::semantic_actions) fn action_from_row(
    row: &Row<'_>,
) -> Result<SemanticAction, rusqlite::Error> {
    let mut attributes = decode_attributes_from_row(row)?;
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
        evidence: evidence::decode(&row.get::<_, Vec<u8>>("evidence_blob")?)?,
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

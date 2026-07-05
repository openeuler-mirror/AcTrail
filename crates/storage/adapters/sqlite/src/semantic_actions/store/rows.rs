use model_core::ids::TraceId;
use model_core::process::{NamespaceIdentity, ProcessIdentity};
use rusqlite::Row;
use semantic_action::{SemanticAction, SemanticActionLink, SemanticEvidence, attr_keys as attrs};

use crate::records::{decode_map, decode_time};
use crate::semantic_actions::codebook::sqlite::{
    decode_completeness, decode_evidence_kind, decode_kind, decode_link_confidence,
    decode_link_role, decode_status,
};
use crate::semantic_actions::cold_fields::decode_text_from_row;

pub(in crate::semantic_actions) fn action_from_row(
    row: &Row<'_>,
) -> Result<SemanticAction, rusqlite::Error> {
    Ok(SemanticAction {
        action_id: row.get("action_id")?,
        trace_id: TraceId::new(row.get("trace_id")?),
        kind: decode_kind(row.get::<_, i64>("kind_code")?)?,
        title: row.get("title")?,
        start_time: decode_time(row.get("start_time")?),
        end_time: row.get::<_, Option<i64>>("end_time")?.map(decode_time),
        process: ProcessIdentity {
            pid: row.get("process_pid")?,
            task_id: row.get("process_task_id")?,
            start_time_ticks: row.get("process_start_ticks")?,
            pid_namespace: row
                .get::<_, Option<String>>("process_pid_namespace")?
                .map(NamespaceIdentity::new),
            generation: row.get("process_generation")?,
        },
        status: decode_status(row.get::<_, i64>("status_code")?)?,
        completeness: decode_completeness(row.get::<_, i64>("completeness_code")?)?,
        confidence_millis: row.get("confidence_millis")?,
        attributes: decode_map(&decode_text_from_row(row, "legacy_attributes")?),
        evidence: Vec::new(),
    })
}

pub(in crate::semantic_actions) fn evidence_from_row(
    row: &Row<'_>,
) -> Result<SemanticEvidence, rusqlite::Error> {
    Ok(SemanticEvidence {
        kind: decode_evidence_kind(row.get::<_, i64>("kind_code")?)?,
        id: row.get("evidence_id")?,
        role: row.get("role").or_else(|_| row.get("evidence_role"))?,
    })
}

pub(super) fn action_link_from_row(row: &Row<'_>) -> Result<SemanticActionLink, rusqlite::Error> {
    let attributes = decode_map(&decode_text_from_row(row, "legacy_attributes")?);
    let valid = row.get::<_, bool>("valid")?
        && !attributes
            .get(attrs::actrail::LINK_VALID)
            .is_some_and(|value| value == "false");
    Ok(SemanticActionLink {
        trace_id: TraceId::new(row.get("trace_id")?),
        parent_action_id: row.get("parent_action_id")?,
        child_action_id: row.get("child_action_id")?,
        role: decode_link_role(row.get::<_, i64>("role_code")?)?,
        confidence: decode_link_confidence(row.get::<_, i64>("confidence_code")?)?,
        valid,
        evidence: Vec::new(),
        attributes,
    })
}

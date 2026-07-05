//! SQLite-facing semantic action storage code helpers.

use rusqlite::Row;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
    SemanticActionStoreError, SemanticEvidenceKind,
};

use crate::semantic_actions::codebook;

pub(in crate::semantic_actions) fn action_kind_code(value: SemanticActionKind) -> i16 {
    codebook::current().action_kind.code(value)
}

pub(in crate::semantic_actions) fn action_kind_code_from_str(
    value: &str,
) -> Result<i16, SemanticActionStoreError> {
    store_code(
        "semantic_action_kind_code",
        codebook::current().action_kind.code_from_str(value),
    )
}

pub(in crate::semantic_actions) fn action_status_code(value: SemanticActionStatus) -> i16 {
    codebook::current().action_status.code(value)
}

pub(in crate::semantic_actions) fn action_completeness_code(
    value: SemanticActionCompleteness,
) -> i16 {
    codebook::current().action_completeness.code(value)
}

pub(in crate::semantic_actions) fn evidence_kind_code(value: SemanticEvidenceKind) -> i16 {
    codebook::current().evidence_kind.code(value)
}

pub(in crate::semantic_actions) fn link_role_code(value: SemanticActionLinkRole) -> i16 {
    codebook::current().link_role.code(value)
}

pub(in crate::semantic_actions) fn link_role_code_from_str(
    value: &str,
) -> Result<i16, SemanticActionStoreError> {
    store_code(
        "semantic_action_link_role_code",
        codebook::current().link_role.code_from_str(value),
    )
}

pub(in crate::semantic_actions) fn link_confidence_code(
    value: SemanticActionLinkConfidence,
) -> i16 {
    codebook::current().link_confidence.code(value)
}

pub(in crate::semantic_actions) fn decode_kind(
    value: i64,
) -> Result<SemanticActionKind, rusqlite::Error> {
    sqlite_code(codebook::current().action_kind.decode(value))
}

pub(in crate::semantic_actions) fn decode_status(
    value: i64,
) -> Result<SemanticActionStatus, rusqlite::Error> {
    sqlite_code(codebook::current().action_status.decode(value))
}

pub(in crate::semantic_actions) fn decode_completeness(
    value: i64,
) -> Result<SemanticActionCompleteness, rusqlite::Error> {
    sqlite_code(codebook::current().action_completeness.decode(value))
}

pub(in crate::semantic_actions) fn decode_evidence_kind(
    value: i64,
) -> Result<SemanticEvidenceKind, rusqlite::Error> {
    sqlite_code(codebook::current().evidence_kind.decode(value))
}

pub(in crate::semantic_actions) fn decode_link_role(
    value: i64,
) -> Result<SemanticActionLinkRole, rusqlite::Error> {
    sqlite_code(codebook::current().link_role.decode(value))
}

pub(in crate::semantic_actions) fn decode_link_confidence(
    value: i64,
) -> Result<SemanticActionLinkConfidence, rusqlite::Error> {
    sqlite_code(codebook::current().link_confidence.decode(value))
}

fn store_code<T>(
    stage: &'static str,
    result: Result<T, codebook::CodebookError>,
) -> Result<T, SemanticActionStoreError> {
    result.map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))
}

fn sqlite_code<T>(result: Result<T, codebook::CodebookError>) -> Result<T, rusqlite::Error> {
    result.map_err(|_| rusqlite::Error::InvalidQuery)
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::semantic_actions) struct LinkEvidenceKey {
    trace_id: u64,
    parent_action_id: String,
    child_action_id: String,
    role_code: i16,
}

impl LinkEvidenceKey {
    pub(in crate::semantic_actions) fn from_link(link: &SemanticActionLink) -> Self {
        Self {
            trace_id: link.trace_id.get(),
            parent_action_id: link.parent_action_id.clone(),
            child_action_id: link.child_action_id.clone(),
            role_code: link_role_code(link.role),
        }
    }

    pub(in crate::semantic_actions) fn from_row(row: &Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            trace_id: row.get("trace_id")?,
            parent_action_id: row.get("parent_action_id")?,
            child_action_id: row.get("child_action_id")?,
            role_code: i16::try_from(row.get::<_, i64>("link_role_code")?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
        })
    }
}

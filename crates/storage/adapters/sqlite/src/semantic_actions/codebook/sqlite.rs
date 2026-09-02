//! SQLite-facing semantic action storage code helpers.

use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkOrigin,
    SemanticActionLinkRole, SemanticActionStatus, SemanticActionStoreError, SemanticEvidenceKind,
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

pub(in crate::semantic_actions) fn link_origin_code(value: SemanticActionLinkOrigin) -> i16 {
    codebook::current().link_origin.code(value)
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

pub(in crate::semantic_actions) fn decode_link_origin(
    value: i64,
) -> Result<SemanticActionLinkOrigin, rusqlite::Error> {
    sqlite_code(codebook::current().link_origin.decode(value))
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

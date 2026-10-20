//! Enum codebook for semantic action SQLite storage.
mod book;
mod current;
mod error;
pub(in crate::semantic_actions) mod evidence_role;
mod kind;
mod links;
mod origin;
pub(in crate::semantic_actions) mod sqlite;

use book::{ActionCompletenessCodes, ActionStatusCodes, EvidenceKindCodes};
pub(crate) use book::{
    CURRENT_SCHEMA_VERSION, SemanticActionCodebook, current, for_schema_version,
};
pub(crate) use error::CodebookError;
use kind::ActionKindCodes;
use links::LinkRoleCodes;
use origin::LinkOriginCodes;

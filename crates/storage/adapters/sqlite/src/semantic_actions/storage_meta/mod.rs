//! SQLite layout metadata for semantic action storage details.

mod current;

pub(in crate::semantic_actions) use current::{ColdFieldMeta, StorageMeta};

pub(crate) const CURRENT_SCHEMA_VERSION: i32 = current::SCHEMA_VERSION;

pub(in crate::semantic_actions) fn current() -> StorageMeta {
    current::CURRENT
}

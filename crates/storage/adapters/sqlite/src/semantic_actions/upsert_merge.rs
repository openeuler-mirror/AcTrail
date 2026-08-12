//! SQLite error mapping for the shared semantic action merge contract.

use semantic_action::{SemanticAction, SemanticActionStoreError};

pub(super) fn merge_action(
    mut existing: SemanticAction,
    incoming: SemanticAction,
) -> Result<SemanticAction, SemanticActionStoreError> {
    existing
        .merge_persistence_update(incoming)
        .map_err(|error| {
            SemanticActionStoreError::new("merge_semantic_action", error.into_message())
        })?;
    Ok(existing)
}

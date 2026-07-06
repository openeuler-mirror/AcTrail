mod hash;

use rusqlite::{OptionalExtension, params};
use semantic_action::SemanticActionStoreError;

pub(in crate::semantic_actions) fn intern_action_id(
    connection: &rusqlite::Connection,
    trace_id: u64,
    action_id: &str,
) -> Result<i64, SemanticActionStoreError> {
    if action_id.is_empty() {
        return Err(SemanticActionStoreError::new(
            "semantic_action_id",
            "action_id must not be empty",
        ));
    }
    let action_id_hash = hash::sha256_hash_blob(action_id.as_bytes());
    connection
        .execute(
            "INSERT OR IGNORE INTO semantic_action_ids (trace_id, action_id, action_id_hash)
             VALUES (?1, ?2, ?3)",
            params![trace_id, action_id, &action_id_hash],
        )
        .map_err(|error| {
            SemanticActionStoreError::new("insert_semantic_action_id", error.to_string())
        })?;
    let row = connection
        .query_row(
            "SELECT action_key, trace_id, action_id_hash
             FROM semantic_action_ids
             WHERE action_id = ?1",
            params![action_id],
            |row| {
                Ok((
                    row.get::<_, i64>("action_key")?,
                    row.get::<_, u64>("trace_id")?,
                    row.get::<_, Vec<u8>>("action_id_hash")?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_semantic_action_id", error.to_string())
        })?
        .ok_or_else(|| {
            SemanticActionStoreError::new(
                "semantic_action_id_missing",
                "action id insert did not materialize a row",
            )
        })?;
    if row.1 == trace_id && row.2 == action_id_hash {
        Ok(row.0)
    } else {
        Err(SemanticActionStoreError::new(
            "semantic_action_id_collision",
            "action_id maps to a different trace or hash",
        ))
    }
}

pub(in crate::semantic_actions) fn resolve_action_key(
    connection: &rusqlite::Connection,
    action_id: &str,
) -> Result<Option<i64>, SemanticActionStoreError> {
    connection
        .query_row(
            "SELECT action_key FROM semantic_action_ids WHERE action_id = ?1",
            params![action_id],
            |row| row.get::<_, i64>("action_key"),
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("resolve_semantic_action_key", error.to_string())
        })
}

pub(in crate::semantic_actions) fn require_action_key(
    connection: &rusqlite::Connection,
    action_id: &str,
) -> Result<i64, SemanticActionStoreError> {
    resolve_action_key(connection, action_id)?.ok_or_else(|| {
        SemanticActionStoreError::new(
            "semantic_action_key_missing",
            format!("missing semantic action id {action_id}"),
        )
    })
}

use rusqlite::{OptionalExtension, params};
use semantic_action::SemanticActionStoreError;

/// Creation normally obtains its key directly from the inserted identity row.
pub(in crate::semantic_actions) fn create_action_id(
    connection: &rusqlite::Connection,
    trace_id: u64,
    action_id: &str,
) -> Result<i64, SemanticActionStoreError> {
    require_non_empty_action_id(action_id)?;
    if let Some(key) = connection
        .prepare_cached(
            "INSERT INTO semantic_action_ids(trace_id, action_id) VALUES (?1, ?2)
         ON CONFLICT(action_id) DO NOTHING RETURNING action_key",
        )
        .and_then(|mut statement| {
            statement.query_row(params![trace_id, action_id], |row| row.get(0))
        })
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("insert_semantic_action_id", error.to_string())
        })?
    {
        return Ok(key);
    }
    let (key, existing_trace): (i64, u64) = connection
        .prepare_cached("SELECT action_key, trace_id FROM semantic_action_ids WHERE action_id=?1")
        .and_then(|mut statement| {
            statement.query_row([action_id], |row| Ok((row.get(0)?, row.get(1)?)))
        })
        .map_err(|error| {
            SemanticActionStoreError::new("read_semantic_action_id", error.to_string())
        })?;
    if existing_trace != trace_id {
        return Err(SemanticActionStoreError::new(
            "semantic_action_id_collision",
            "action_id maps to a different trace",
        ));
    }
    Ok(key)
}

pub(in crate::semantic_actions) fn intern_action_id(
    connection: &mut rusqlite::Connection,
    trace_id: u64,
    action_id: &str,
) -> Result<i64, SemanticActionStoreError> {
    require_non_empty_action_id(action_id)?;
    // Link endpoints and content records commonly refer to an existing identity.
    // The action_id column is UNIQUE, so a matching trace + action_id is ours.
    if let Some(action_key) = connection
        .prepare_cached(
            "SELECT action_key FROM semantic_action_ids
             WHERE trace_id = ?1 AND action_id = ?2",
        )
        .and_then(|mut statement| {
            statement.query_row(params![trace_id, action_id], |row| {
                row.get::<_, i64>("action_key")
            })
        })
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("resolve_semantic_action_key", error.to_string())
        })?
    {
        return Ok(action_key);
    }
    intern_action_id_slow(connection, trace_id, action_id)
}

/// Shared-borrow variant for cold paths that already hold `&Connection`.
pub(in crate::semantic_actions) fn intern_action_id_shared(
    connection: &rusqlite::Connection,
    trace_id: u64,
    action_id: &str,
) -> Result<i64, SemanticActionStoreError> {
    require_non_empty_action_id(action_id)?;
    intern_action_id_slow(connection, trace_id, action_id)
}

fn intern_action_id_slow(
    connection: &rusqlite::Connection,
    trace_id: u64,
    action_id: &str,
) -> Result<i64, SemanticActionStoreError> {
    connection
        .execute(
            "INSERT OR IGNORE INTO semantic_action_ids (trace_id, action_id)
             VALUES (?1, ?2)",
            params![trace_id, action_id],
        )
        .map_err(|error| {
            SemanticActionStoreError::new("insert_semantic_action_id", error.to_string())
        })?;
    let row = connection
        .query_row(
            "SELECT action_key, trace_id
             FROM semantic_action_ids
             WHERE action_id = ?1",
            params![action_id],
            |row| {
                Ok((
                    row.get::<_, i64>("action_key")?,
                    row.get::<_, u64>("trace_id")?,
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
    if row.1 == trace_id {
        Ok(row.0)
    } else {
        Err(SemanticActionStoreError::new(
            "semantic_action_id_collision",
            "action_id maps to a different trace",
        ))
    }
}

fn require_non_empty_action_id(action_id: &str) -> Result<(), SemanticActionStoreError> {
    if action_id.is_empty() {
        return Err(SemanticActionStoreError::new(
            "semantic_action_id",
            "action_id must not be empty",
        ));
    }
    Ok(())
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

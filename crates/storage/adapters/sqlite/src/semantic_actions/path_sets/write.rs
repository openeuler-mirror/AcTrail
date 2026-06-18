use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};
use semantic_action::{FilePathSetWrite, SemanticActionStoreError};

use super::hash::{encode_path_ids, stable_hash_bytes, stable_hash_text};

pub(in crate::semantic_actions) fn upsert_file_path_sets(
    connection: &rusqlite::Connection,
    path_sets: &[FilePathSetWrite],
) -> Result<(), SemanticActionStoreError> {
    for path_set in path_sets {
        upsert_file_path_set(connection, path_set)?;
    }
    Ok(())
}

fn upsert_file_path_set(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
) -> Result<(), SemanticActionStoreError> {
    if path_set.chunk_max_paths == 0 {
        return Err(SemanticActionStoreError::new(
            "file_path_set_chunk_config",
            "chunk_max_paths must be greater than zero",
        ));
    }
    let unique_paths = path_set
        .paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if unique_paths.len() != path_set.paths.len()
        || path_set.stored_path_count != path_set.paths.len() as u64
    {
        return Err(SemanticActionStoreError::new(
            "file_path_set_paths",
            "path set paths must be unique and match stored_path_count",
        ));
    }
    let mut path_ids = Vec::with_capacity(path_set.paths.len());
    for path in &path_set.paths {
        path_ids.push(intern_path(connection, path_set.trace_id.get(), path)?);
    }
    path_ids.sort_unstable();
    let chunk_ids = upsert_chunks(connection, path_set, &path_ids)?;
    write_path_set_row(connection, path_set)?;
    replace_chunk_refs(connection, path_set, &chunk_ids)
}

fn intern_path(
    connection: &rusqlite::Connection,
    trace_id: u64,
    path: &str,
) -> Result<u64, SemanticActionStoreError> {
    let path_hash = stable_hash_text(path);
    connection
        .execute(
            "INSERT OR IGNORE INTO file_paths (trace_id, path_hash, path_text)
             VALUES (?1, ?2, ?3)",
            params![trace_id, &path_hash, path],
        )
        .map_err(|error| SemanticActionStoreError::new("insert_file_path", error.to_string()))?;
    let path_id = connection
        .query_row(
            "SELECT path_id FROM file_paths
             WHERE trace_id = ?1 AND path_hash = ?2 AND path_text = ?3",
            params![trace_id, &path_hash, path],
            |row| {
                let value = row.get::<_, i64>("path_id")?;
                u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
            },
        )
        .map_err(|error| SemanticActionStoreError::new("read_file_path_id", error.to_string()))?;
    Ok(path_id)
}

fn upsert_chunks(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
    path_ids: &[u64],
) -> Result<Vec<String>, SemanticActionStoreError> {
    let mut chunk_ids = Vec::new();
    for chunk in path_ids.chunks(path_set.chunk_max_paths as usize) {
        let encoded = encode_path_ids(chunk);
        let hash_input = format!("{}\n{}", path_set.chunking_scheme, encoded);
        let chunk_hash = stable_hash_bytes(hash_input.as_bytes());
        let chunk_id = format!(
            "trace:{}:file-path-chunk:{}",
            path_set.trace_id.get(),
            chunk_hash
        );
        connection
            .execute(
                "INSERT OR IGNORE INTO file_path_set_chunks (
                    trace_id, chunk_id, chunk_hash, item_count,
                    encoded_sorted_path_ids, chunking_scheme
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    path_set.trace_id.get(),
                    &chunk_id,
                    &chunk_hash,
                    to_i64(chunk.len(), "file_path_set_chunk_item_count")?,
                    &encoded,
                    &path_set.chunking_scheme,
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("insert_file_path_set_chunk", error.to_string())
            })?;
        verify_chunk_identity(connection, path_set.trace_id.get(), &chunk_id, &encoded)?;
        chunk_ids.push(chunk_id);
    }
    Ok(chunk_ids)
}

fn verify_chunk_identity(
    connection: &rusqlite::Connection,
    trace_id: u64,
    chunk_id: &str,
    encoded: &str,
) -> Result<(), SemanticActionStoreError> {
    let existing = connection
        .query_row(
            "SELECT encoded_sorted_path_ids FROM file_path_set_chunks
             WHERE trace_id = ?1 AND chunk_id = ?2",
            params![trace_id, chunk_id],
            |row| row.get::<_, String>("encoded_sorted_path_ids"),
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_file_path_set_chunk", error.to_string())
        })?;
    match existing {
        Some(existing) if existing == encoded => Ok(()),
        Some(_) => Err(SemanticActionStoreError::new(
            "file_path_set_chunk_hash_collision",
            "chunk hash collision changed encoded path ids",
        )),
        None => Err(SemanticActionStoreError::new(
            "file_path_set_chunk_missing",
            "chunk insert did not materialize a row",
        )),
    }
}

fn write_path_set_row(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
) -> Result<(), SemanticActionStoreError> {
    connection
        .execute(
            "INSERT INTO file_path_sets (
                trace_id, path_set_id, action_id, state, unique_path_count,
                stored_path_count, chunking_scheme
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(trace_id, path_set_id) DO UPDATE SET
                action_id = excluded.action_id,
                state = excluded.state,
                unique_path_count = excluded.unique_path_count,
                stored_path_count = excluded.stored_path_count,
                chunking_scheme = excluded.chunking_scheme",
            params![
                path_set.trace_id.get(),
                &path_set.path_set_id,
                &path_set.action_id,
                path_set.state.as_str(),
                to_i64(
                    path_set.unique_path_count,
                    "file_path_set_unique_path_count"
                )?,
                to_i64(
                    path_set.stored_path_count,
                    "file_path_set_stored_path_count"
                )?,
                &path_set.chunking_scheme,
            ],
        )
        .map(|_| ())
        .map_err(|error| SemanticActionStoreError::new("upsert_file_path_set", error.to_string()))
}

fn replace_chunk_refs(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
    chunk_ids: &[String],
) -> Result<(), SemanticActionStoreError> {
    connection
        .execute(
            "DELETE FROM file_path_set_chunk_refs
             WHERE trace_id = ?1 AND path_set_id = ?2",
            params![path_set.trace_id.get(), &path_set.path_set_id],
        )
        .map_err(|error| {
            SemanticActionStoreError::new("delete_file_path_set_chunk_refs", error.to_string())
        })?;
    for (index, chunk_id) in chunk_ids.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO file_path_set_chunk_refs (
                    trace_id, path_set_id, chunk_order, chunk_id
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    path_set.trace_id.get(),
                    &path_set.path_set_id,
                    to_i64(index, "file_path_set_chunk_order")?,
                    chunk_id,
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("insert_file_path_set_chunk_ref", error.to_string())
            })?;
    }
    Ok(())
}

fn to_i64(value: impl TryInto<i64>, stage: &'static str) -> Result<i64, SemanticActionStoreError> {
    value
        .try_into()
        .map_err(|_| SemanticActionStoreError::new(stage, "value exceeds i64"))
}

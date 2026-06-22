use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};
use semantic_action::{
    FilePathSetState, FilePathSetWrite, SemanticActionStoreError, file_path_set_identity_for_paths,
};

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
    if path_set.state == FilePathSetState::Overflow
        && existing_overflow_path_set_matches(connection, path_set)?
    {
        return write_action_ref(connection, path_set);
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
    validate_path_set_identity(path_set)?;
    write_path_set_row(connection, path_set)?;
    write_chunk_refs_once(connection, path_set, &chunk_ids)?;
    write_action_ref(connection, path_set)
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
    let identity = file_path_set_identity_for_paths(
        path_set.state,
        &path_set.chunking_scheme,
        path_set.paths.iter().map(String::as_str),
    );
    let path_set_hash = match path_set.state {
        FilePathSetState::Complete => identity.path_set_hash,
        FilePathSetState::Pending | FilePathSetState::Overflow => {
            stable_hash_text(&path_set.path_set_id)
        }
    };
    connection
        .execute(
            "INSERT INTO file_path_sets (
                trace_id, path_set_id, path_set_hash, state, unique_path_count,
                stored_path_count, chunking_scheme
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(trace_id, path_set_id) DO NOTHING",
            params![
                path_set.trace_id.get(),
                &path_set.path_set_id,
                &path_set_hash,
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
        .map_err(|error| {
            SemanticActionStoreError::new("insert_file_path_set", error.to_string())
        })?;
    verify_path_set_row(connection, path_set, &path_set_hash)
}

fn validate_path_set_identity(path_set: &FilePathSetWrite) -> Result<(), SemanticActionStoreError> {
    if path_set.state != FilePathSetState::Complete {
        return Ok(());
    }
    let identity = file_path_set_identity_for_paths(
        path_set.state,
        &path_set.chunking_scheme,
        path_set.paths.iter().map(String::as_str),
    );
    if path_set.path_set_id == identity.path_set_id {
        return Ok(());
    }
    Err(SemanticActionStoreError::new(
        "file_path_set_identity",
        format!(
            "complete path set id {} does not match canonical id {}",
            path_set.path_set_id, identity.path_set_id
        ),
    ))
}

fn existing_overflow_path_set_matches(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
) -> Result<bool, SemanticActionStoreError> {
    let existing = connection
        .query_row(
            "SELECT state, chunking_scheme
             FROM file_path_sets
             WHERE trace_id = ?1 AND path_set_id = ?2",
            params![path_set.trace_id.get(), &path_set.path_set_id],
            |row| {
                Ok((
                    row.get::<_, String>("state")?,
                    row.get::<_, String>("chunking_scheme")?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_overflow_path_set", error.to_string())
        })?;
    let Some((state, chunking_scheme)) = existing else {
        return Ok(false);
    };
    if state == FilePathSetState::Overflow.as_str() && chunking_scheme == path_set.chunking_scheme {
        return Ok(true);
    }
    Err(SemanticActionStoreError::new(
        "file_path_set_hash_collision",
        "overflow path set identity changed state or chunking scheme",
    ))
}

fn verify_path_set_row(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
    path_set_hash: &str,
) -> Result<(), SemanticActionStoreError> {
    let existing = connection
        .query_row(
            "SELECT path_set_hash, state, unique_path_count, stored_path_count, chunking_scheme
             FROM file_path_sets
             WHERE trace_id = ?1 AND path_set_id = ?2",
            params![path_set.trace_id.get(), &path_set.path_set_id],
            |row| {
                Ok((
                    row.get::<_, String>("path_set_hash")?,
                    row.get::<_, String>("state")?,
                    row.get::<_, i64>("unique_path_count")?,
                    row.get::<_, i64>("stored_path_count")?,
                    row.get::<_, String>("chunking_scheme")?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_file_path_set_identity", error.to_string())
        })?;
    let Some((existing_hash, state, unique_path_count, stored_path_count, chunking_scheme)) =
        existing
    else {
        return Err(SemanticActionStoreError::new(
            "file_path_set_missing",
            "path set insert did not materialize a row",
        ));
    };
    let expected_unique = to_i64(
        path_set.unique_path_count,
        "file_path_set_unique_path_count",
    )?;
    let expected_stored = to_i64(
        path_set.stored_path_count,
        "file_path_set_stored_path_count",
    )?;
    if existing_hash == path_set_hash
        && state == path_set.state.as_str()
        && unique_path_count == expected_unique
        && stored_path_count == expected_stored
        && chunking_scheme == path_set.chunking_scheme
    {
        return Ok(());
    }
    Err(SemanticActionStoreError::new(
        "file_path_set_hash_collision",
        "path set identity changed row metadata",
    ))
}

fn write_chunk_refs_once(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
    chunk_ids: &[String],
) -> Result<(), SemanticActionStoreError> {
    let existing = read_chunk_refs(connection, path_set)?;
    if !existing.is_empty() {
        if existing == chunk_ids {
            return Ok(());
        }
        return Err(SemanticActionStoreError::new(
            "file_path_set_hash_collision",
            "path set identity changed chunk refs",
        ));
    }
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

fn read_chunk_refs(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
) -> Result<Vec<String>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT chunk_id
             FROM file_path_set_chunk_refs
             WHERE trace_id = ?1 AND path_set_id = ?2
             ORDER BY chunk_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new("prepare_file_path_set_chunk_refs", error.to_string())
        })?;
    let rows = statement
        .query_map(
            params![path_set.trace_id.get(), &path_set.path_set_id],
            |row| row.get::<_, String>("chunk_id"),
        )
        .map_err(|error| {
            SemanticActionStoreError::new("query_file_path_set_chunk_refs", error.to_string())
        })?;
    rows.map(|row| {
        row.map_err(|error| {
            SemanticActionStoreError::new("map_file_path_set_chunk_refs", error.to_string())
        })
    })
    .collect()
}

fn write_action_ref(
    connection: &rusqlite::Connection,
    path_set: &FilePathSetWrite,
) -> Result<(), SemanticActionStoreError> {
    connection
        .execute(
            "INSERT INTO file_path_set_action_refs (
                trace_id, action_id, path_set_id
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(trace_id, action_id) DO UPDATE SET
                path_set_id = excluded.path_set_id",
            params![
                path_set.trace_id.get(),
                &path_set.action_id,
                &path_set.path_set_id,
            ],
        )
        .map(|_| ())
        .map_err(|error| {
            SemanticActionStoreError::new("upsert_file_path_set_action_ref", error.to_string())
        })
}

fn to_i64(value: impl TryInto<i64>, stage: &'static str) -> Result<i64, SemanticActionStoreError> {
    value
        .try_into()
        .map_err(|_| SemanticActionStoreError::new(stage, "value exceeds i64"))
}

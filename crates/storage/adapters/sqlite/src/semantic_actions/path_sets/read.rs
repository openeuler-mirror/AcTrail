use rusqlite::{OptionalExtension, params};
use semantic_action::{
    FilePathSetPath, FilePathSetPathPage, FilePathSetState, SemanticActionStoreError,
};

use model_core::ids::TraceId;

pub(in crate::semantic_actions) fn file_path_set_paths_page(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    action_id: &str,
    offset: usize,
    limit: usize,
) -> Result<Option<FilePathSetPathPage>, SemanticActionStoreError> {
    let Some(row) = read_path_set_row(connection, trace_id, action_id)? else {
        return Ok(None);
    };
    let path_ids = read_path_set_path_ids(connection, trace_id, &row.path_set_id)?;
    let total_count = path_ids.len();
    let selected_ids = path_ids
        .iter()
        .skip(offset)
        .take(limit)
        .copied()
        .collect::<Vec<_>>();
    let paths = read_paths(connection, trace_id, &selected_ids)?;
    Ok(Some(FilePathSetPathPage {
        path_set_id: row.path_set_id,
        action_id: row.action_id,
        state: row.state,
        unique_path_count: row.unique_path_count,
        stored_path_count: row.stored_path_count,
        chunking_scheme: row.chunking_scheme,
        paths,
        total_count,
    }))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PathSetRow {
    path_set_id: String,
    action_id: String,
    state: FilePathSetState,
    unique_path_count: u64,
    stored_path_count: u64,
    chunking_scheme: String,
}

fn read_path_set_row(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    action_id: &str,
) -> Result<Option<PathSetRow>, SemanticActionStoreError> {
    connection
        .query_row(
            "SELECT path_sets.path_set_id, ids.action_id, path_sets.state,
                    path_sets.unique_path_count, path_sets.stored_path_count,
                    path_sets.chunking_scheme
             FROM file_path_set_action_refs action_refs
             JOIN semantic_action_ids ids
               ON ids.action_key = action_refs.action_key
             JOIN file_path_sets path_sets
               ON path_sets.trace_id = action_refs.trace_id
              AND path_sets.path_set_id = action_refs.path_set_id
             WHERE action_refs.trace_id = ?1 AND ids.action_id = ?2",
            params![trace_id.get(), action_id],
            |row| {
                let state = FilePathSetState::parse(&row.get::<_, String>("state")?)
                    .ok_or(rusqlite::Error::InvalidQuery)?;
                Ok(PathSetRow {
                    path_set_id: row.get("path_set_id")?,
                    action_id: row.get("action_id")?,
                    state,
                    unique_path_count: u64_from_i64(row.get::<_, i64>("unique_path_count")?)?,
                    stored_path_count: u64_from_i64(row.get::<_, i64>("stored_path_count")?)?,
                    chunking_scheme: row.get("chunking_scheme")?,
                })
            },
        )
        .optional()
        .map_err(|error| SemanticActionStoreError::new("read_file_path_set", error.to_string()))
}

fn read_path_set_path_ids(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    path_set_id: &str,
) -> Result<Vec<u64>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT chunks.encoded_sorted_path_ids
             FROM file_path_set_chunk_refs refs
             JOIN file_path_set_chunks chunks
               ON chunks.trace_id = refs.trace_id
              AND chunks.chunk_id = refs.chunk_id
             WHERE refs.trace_id = ?1 AND refs.path_set_id = ?2
             ORDER BY refs.chunk_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new("prepare_file_path_set_chunks", error.to_string())
        })?;
    let rows = statement
        .query_map(params![trace_id.get(), path_set_id], |row| {
            row.get::<_, String>("encoded_sorted_path_ids")
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_file_path_set_chunks", error.to_string())
        })?;
    let mut path_ids = Vec::new();
    for row in rows {
        let encoded = row.map_err(|error| {
            SemanticActionStoreError::new("map_file_path_set_chunks", error.to_string())
        })?;
        path_ids.extend(decode_path_ids(&encoded)?);
    }
    Ok(path_ids)
}

fn read_paths(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    path_ids: &[u64],
) -> Result<Vec<FilePathSetPath>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare("SELECT path_text FROM file_paths WHERE trace_id = ?1 AND path_id = ?2")
        .map_err(|error| SemanticActionStoreError::new("prepare_file_paths", error.to_string()))?;
    let mut paths = Vec::new();
    for path_id in path_ids {
        let path = statement
            .query_row(params![trace_id.get(), path_id], |row| {
                row.get::<_, String>("path_text")
            })
            .map_err(|error| SemanticActionStoreError::new("read_file_path", error.to_string()))?;
        paths.push(FilePathSetPath {
            path_id: *path_id,
            path,
        });
    }
    Ok(paths)
}

fn decode_path_ids(value: &str) -> Result<Vec<u64>, SemanticActionStoreError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|item| {
            item.parse::<u64>().map_err(|error| {
                SemanticActionStoreError::new("decode_file_path_id", error.to_string())
            })
        })
        .collect()
}

fn u64_from_i64(value: i64) -> Result<u64, rusqlite::Error> {
    u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}

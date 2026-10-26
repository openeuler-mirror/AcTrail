use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension, params};
use semantic_action::{
    LlmRequestBlock, LlmRequestBlockRef, LlmRequestContentWrite, SemanticActionStoreError,
};

use crate::semantic_actions::action_ids::require_action_key;

const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;

pub(in crate::semantic_actions) fn upsert_llm_request_contents(
    connection: &rusqlite::Connection,
    contents: &[LlmRequestContentWrite],
) -> Result<(), SemanticActionStoreError> {
    for content in contents {
        upsert_llm_request_content(connection, content)?;
    }
    Ok(())
}

fn upsert_llm_request_content(
    connection: &rusqlite::Connection,
    content: &LlmRequestContentWrite,
) -> Result<(), SemanticActionStoreError> {
    validate_content_shape(content)?;
    let action_key = require_action(connection, content)?;
    let mut blocks = BlockWriter::new(connection, content.manifest.trace_id.get());
    for block in &content.blocks {
        blocks.resolve(block)?;
    }
    let manifest_id = write_manifest_once(connection, action_key, content)?;
    let expected_refs = blocks.references(&content.block_refs)?;
    write_refs_once(connection, manifest_id, &expected_refs)
}

fn validate_content_shape(
    content: &LlmRequestContentWrite,
) -> Result<(), SemanticActionStoreError> {
    let manifest = &content.manifest;
    if manifest.action_id.is_empty() {
        return Err(SemanticActionStoreError::new(
            "llm_request_content_action_id",
            "action_id must not be empty",
        ));
    }
    let mut provided_blocks = BTreeSet::new();
    for block in &content.blocks {
        if block.trace_id != manifest.trace_id {
            return Err(SemanticActionStoreError::new(
                "llm_request_content_trace",
                "block trace_id does not match manifest trace_id",
            ));
        }
        if block.encoded_bytes.len() as u64 != block.uncompressed_bytes {
            return Err(SemanticActionStoreError::new(
                "llm_request_block_size",
                "encoded bytes must match uncompressed bytes for canonical-json-v1 blocks",
            ));
        }
        if !provided_blocks.insert(block.block_hash.as_str()) {
            return Err(SemanticActionStoreError::new(
                "llm_request_block_duplicate",
                "content must provide each block identity exactly once",
            ));
        }
    }
    for (index, block_ref) in content.block_refs.iter().enumerate() {
        if block_ref.trace_id != manifest.trace_id || block_ref.action_id != manifest.action_id {
            return Err(SemanticActionStoreError::new(
                "llm_request_block_ref_owner",
                "block ref owner does not match manifest",
            ));
        }
        if block_ref.ordinal != index as u32 {
            return Err(SemanticActionStoreError::new(
                "llm_request_block_ref_ordinal",
                "block ref ordinals must be contiguous from zero",
            ));
        }
        if !provided_blocks.contains(block_ref.block_hash.as_str()) {
            return Err(SemanticActionStoreError::new(
                "llm_request_ref_block_missing",
                "content must provide every referenced block",
            ));
        }
    }
    Ok(())
}

fn require_action(
    connection: &rusqlite::Connection,
    content: &LlmRequestContentWrite,
) -> Result<i64, SemanticActionStoreError> {
    let action_key = require_action_key(connection, &content.manifest.action_id)?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM semantic_actions
             WHERE trace_id = ?1 AND action_key = ?2",
            params![content.manifest.trace_id.get(), action_key],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_action", error.to_string())
        })?
        .is_some();
    if exists {
        Ok(action_key)
    } else {
        Err(SemanticActionStoreError::new(
            "llm_request_action_missing",
            "cannot write LLM request content before its semantic action",
        ))
    }
}

/// IDs resolved within one content write. The caller's existing write transaction
/// covers lookup and insertion; SQLite owns the persistent block identity.
struct BlockWriter<'a> {
    connection: &'a rusqlite::Connection,
    trace_id: u64,
    ids: BTreeMap<&'a str, i64>,
}

impl<'a> BlockWriter<'a> {
    fn new(connection: &'a rusqlite::Connection, trace_id: u64) -> Self {
        Self {
            connection,
            trace_id,
            ids: BTreeMap::new(),
        }
    }

    fn resolve(&mut self, block: &'a LlmRequestBlock) -> Result<(), SemanticActionStoreError> {
        let block_hash = sha256_hash_blob(&block.block_hash, "llm_request_block_hash")?;
        let expected_size = to_i64(
            block.uncompressed_bytes,
            "llm_request_block_uncompressed_bytes",
        )?;
        let existing = self
            .connection
            .query_row(
                "SELECT block_id, uncompressed_bytes FROM llm_request_blocks
                 WHERE trace_id = ?1 AND block_hash = ?2",
                params![self.trace_id, &block_hash],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_llm_request_block", error.to_string())
            })?;
        let block_id = if let Some((block_id, stored_size)) = existing {
            if stored_size != expected_size {
                return Err(SemanticActionStoreError::new(
                    "llm_request_block_size_conflict",
                    "same block identity has a different byte length",
                ));
            }
            block_id
        } else {
            self.connection
                .query_row(
                    "INSERT INTO llm_request_blocks (
                        trace_id, block_hash, uncompressed_bytes, encoded_bytes
                     ) VALUES (?1, ?2, ?3, ?4) RETURNING block_id",
                    params![
                        self.trace_id,
                        &block_hash,
                        expected_size,
                        &block.encoded_bytes
                    ],
                    |row| row.get(0),
                )
                .map_err(|error| {
                    SemanticActionStoreError::new("insert_llm_request_block", error.to_string())
                })?
        };
        self.ids.insert(&block.block_hash, block_id);
        Ok(())
    }

    fn references(
        &self,
        refs: &[LlmRequestBlockRef],
    ) -> Result<Vec<(u32, i64)>, SemanticActionStoreError> {
        refs.iter()
            .map(|block_ref| {
                self.ids
                    .get(block_ref.block_hash.as_str())
                    .map(|block_id| (block_ref.ordinal, *block_id))
                    .ok_or_else(|| {
                        SemanticActionStoreError::new(
                            "llm_request_ref_block_missing",
                            "block reference has no resolved ID",
                        )
                    })
            })
            .collect()
    }
}

fn write_manifest_once(
    connection: &rusqlite::Connection,
    action_key: i64,
    content: &LlmRequestContentWrite,
) -> Result<i64, SemanticActionStoreError> {
    let manifest = &content.manifest;
    let existing = connection
        .query_row(
            "SELECT manifest_id, format_version, skeleton_json
             FROM llm_request_manifests
             WHERE trace_id = ?1 AND action_key = ?2",
            params![manifest.trace_id.get(), action_key],
            |row| {
                Ok((
                    row.get::<_, i64>("manifest_id")?,
                    row.get::<_, i64>("format_version")?,
                    row.get::<_, String>("skeleton_json")?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_manifest", error.to_string())
        })?;
    if let Some((manifest_id, format_version, skeleton)) = existing {
        let expected_version = to_i64(manifest.format_version, "llm_request_format_version")?;
        if format_version == expected_version && skeleton == manifest.skeleton_json {
            return Ok(manifest_id);
        }
        return Err(SemanticActionStoreError::new(
            "llm_request_manifest_conflict",
            "same action_id already has different LLM request manifest",
        ));
    }
    connection
        .execute(
            "INSERT INTO llm_request_manifests (
                trace_id, action_key, format_version, skeleton_json
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                manifest.trace_id.get(),
                action_key,
                to_i64(manifest.format_version, "llm_request_format_version")?,
                &manifest.skeleton_json,
            ],
        )
        .map_err(|error| {
            SemanticActionStoreError::new("insert_llm_request_manifest", error.to_string())
        })?;
    Ok(connection.last_insert_rowid())
}

fn write_refs_once(
    connection: &rusqlite::Connection,
    manifest_id: i64,
    expected: &[(u32, i64)],
) -> Result<(), SemanticActionStoreError> {
    let existing = read_refs(connection, manifest_id)?;
    if !existing.is_empty() {
        if existing == expected {
            return Ok(());
        }
        return Err(SemanticActionStoreError::new(
            "llm_request_block_refs_conflict",
            "same action_id already has different LLM request block refs",
        ));
    }
    for (ordinal, block_id) in expected {
        connection
            .execute(
                "INSERT INTO llm_request_block_refs (
                    manifest_id, ordinal, block_id
                 ) VALUES (?1, ?2, ?3)",
                params![
                    manifest_id,
                    to_i64(*ordinal, "llm_request_block_ref_ordinal")?,
                    block_id,
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("insert_llm_request_block_ref", error.to_string())
            })?;
    }
    Ok(())
}

fn read_refs(
    connection: &rusqlite::Connection,
    manifest_id: i64,
) -> Result<Vec<(u32, i64)>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT ordinal, block_id
             FROM llm_request_block_refs
             WHERE manifest_id = ?1
             ORDER BY ordinal ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new("prepare_llm_request_block_refs", error.to_string())
        })?;
    let rows = statement
        .query_map(params![manifest_id], |row| {
            Ok((
                row.get::<_, u32>("ordinal")?,
                row.get::<_, i64>("block_id")?,
            ))
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_llm_request_block_refs", error.to_string())
        })?;
    rows.map(|row| {
        row.map_err(|error| {
            SemanticActionStoreError::new("map_llm_request_block_refs", error.to_string())
        })
    })
    .collect()
}

fn sha256_hash_blob(hash: &str, stage: &'static str) -> Result<Vec<u8>, SemanticActionStoreError> {
    let Some(hex) = hash.strip_prefix(SHA256_PREFIX) else {
        return Err(SemanticActionStoreError::new(
            stage,
            "hash must use sha256:<64 lowercase hex> format",
        ));
    };
    if hex.len() != SHA256_HEX_LEN {
        return Err(SemanticActionStoreError::new(
            stage,
            "sha256 hash must contain exactly 64 hex characters",
        ));
    }
    let mut bytes = Vec::with_capacity(32);
    let raw = hex.as_bytes();
    for pair in raw.chunks_exact(2) {
        let high = hex_value(pair[0]).ok_or_else(|| {
            SemanticActionStoreError::new(stage, "sha256 hash contains a non-hex character")
        })?;
        let low = hex_value(pair[1]).ok_or_else(|| {
            SemanticActionStoreError::new(stage, "sha256 hash contains a non-hex character")
        })?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn to_i64(value: impl TryInto<i64>, stage: &'static str) -> Result<i64, SemanticActionStoreError> {
    value
        .try_into()
        .map_err(|_| SemanticActionStoreError::new(stage, "value exceeds i64"))
}

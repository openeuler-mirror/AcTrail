use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};
use semantic_action::{LlmRequestContentWrite, SemanticActionStoreError};

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
    require_action(connection, content)?;
    for block in &content.blocks {
        upsert_block(connection, block)?;
    }
    let manifest_id = write_manifest_once(connection, content)?;
    let expected_refs = content
        .block_refs
        .iter()
        .map(|block_ref| {
            let block_id =
                require_block_id(connection, block_ref.trace_id.get(), &block_ref.block_hash)?;
            Ok((block_ref.ordinal, block_id))
        })
        .collect::<Result<Vec<_>, SemanticActionStoreError>>()?;
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
        provided_blocks.insert(block.block_hash.clone());
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
        if !provided_blocks.contains(&block_ref.block_hash) {
            continue;
        }
    }
    Ok(())
}

fn require_action(
    connection: &rusqlite::Connection,
    content: &LlmRequestContentWrite,
) -> Result<(), SemanticActionStoreError> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM semantic_actions
             WHERE trace_id = ?1 AND action_id = ?2",
            params![content.manifest.trace_id.get(), &content.manifest.action_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_action", error.to_string())
        })?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(SemanticActionStoreError::new(
            "llm_request_action_missing",
            "cannot write LLM request content before its semantic action",
        ))
    }
}

fn upsert_block(
    connection: &rusqlite::Connection,
    block: &semantic_action::LlmRequestBlock,
) -> Result<(), SemanticActionStoreError> {
    let block_hash = sha256_hash_blob(&block.block_hash, "llm_request_block_hash")?;
    connection
        .execute(
            "INSERT OR IGNORE INTO llm_request_blocks (
                trace_id, block_hash, uncompressed_bytes, encoded_bytes
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                block.trace_id.get(),
                &block_hash,
                to_i64(
                    block.uncompressed_bytes,
                    "llm_request_block_uncompressed_bytes"
                )?,
                &block.encoded_bytes,
            ],
        )
        .map_err(|error| {
            SemanticActionStoreError::new("insert_llm_request_block", error.to_string())
        })?;
    verify_block(connection, block, &block_hash)
}

fn verify_block(
    connection: &rusqlite::Connection,
    block: &semantic_action::LlmRequestBlock,
    block_hash: &[u8],
) -> Result<(), SemanticActionStoreError> {
    let existing = connection
        .query_row(
            "SELECT uncompressed_bytes, encoded_bytes
             FROM llm_request_blocks
             WHERE trace_id = ?1 AND block_hash = ?2",
            params![block.trace_id.get(), block_hash],
            |row| {
                Ok((
                    row.get::<_, i64>("uncompressed_bytes")?,
                    row.get::<_, Vec<u8>>("encoded_bytes")?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_block", error.to_string())
        })?;
    let Some((uncompressed_bytes, encoded_bytes)) = existing else {
        return Err(SemanticActionStoreError::new(
            "llm_request_block_missing",
            "block insert did not materialize a row",
        ));
    };
    if uncompressed_bytes
        == to_i64(
            block.uncompressed_bytes,
            "llm_request_block_uncompressed_bytes",
        )?
        && encoded_bytes == block.encoded_bytes
    {
        return Ok(());
    }
    Err(SemanticActionStoreError::new(
        "llm_request_block_hash_collision",
        "block hash collision changed canonical block bytes",
    ))
}

fn require_block_id(
    connection: &rusqlite::Connection,
    trace_id: u64,
    block_hash: &str,
) -> Result<i64, SemanticActionStoreError> {
    let block_hash_blob = sha256_hash_blob(block_hash, "llm_request_ref_block_hash")?;
    connection
        .query_row(
            "SELECT block_id FROM llm_request_blocks
             WHERE trace_id = ?1 AND block_hash = ?2",
            params![trace_id, &block_hash_blob],
            |row| row.get::<_, i64>("block_id"),
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_ref_block", error.to_string())
        })?
        .ok_or_else(|| {
            SemanticActionStoreError::new(
                "llm_request_ref_block_missing",
                format!("block ref points at missing block {block_hash}"),
            )
        })
}

fn write_manifest_once(
    connection: &rusqlite::Connection,
    content: &LlmRequestContentWrite,
) -> Result<i64, SemanticActionStoreError> {
    let manifest = &content.manifest;
    let expected_hash = sha256_hash_blob(&manifest.canonical_body_hash, "llm_request_body_hash")?;
    let existing = connection
        .query_row(
            "SELECT manifest_id, format_version, canonical_body_hash,
                    canonical_body_bytes, skeleton_json
             FROM llm_request_manifests
             WHERE trace_id = ?1 AND action_id = ?2",
            params![manifest.trace_id.get(), &manifest.action_id],
            |row| {
                Ok((
                    row.get::<_, i64>("manifest_id")?,
                    row.get::<_, i64>("format_version")?,
                    row.get::<_, Vec<u8>>("canonical_body_hash")?,
                    row.get::<_, i64>("canonical_body_bytes")?,
                    row.get::<_, String>("skeleton_json")?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_manifest", error.to_string())
        })?;
    if let Some((manifest_id, format_version, hash, bytes, skeleton)) = existing {
        let expected_version = to_i64(manifest.format_version, "llm_request_format_version")?;
        let expected_bytes = to_i64(manifest.canonical_body_bytes, "llm_request_body_bytes")?;
        if format_version == expected_version
            && hash == expected_hash
            && bytes == expected_bytes
            && skeleton == manifest.skeleton_json
        {
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
                trace_id, action_id, format_version, canonical_body_hash,
                canonical_body_bytes, skeleton_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                manifest.trace_id.get(),
                &manifest.action_id,
                to_i64(manifest.format_version, "llm_request_format_version")?,
                &expected_hash,
                to_i64(manifest.canonical_body_bytes, "llm_request_body_bytes")?,
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

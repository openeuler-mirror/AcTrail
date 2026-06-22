use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::{OptionalExtension, params};
use semantic_action::{LlmRequestContentPage, SemanticActionStoreError};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const BLOCK_PLACEHOLDER_KEY: &str = "$actrail_llm_block";

pub(in crate::semantic_actions) fn llm_request_content_page(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    action_id: &str,
    max_bytes: usize,
) -> Result<Option<LlmRequestContentPage>, SemanticActionStoreError> {
    let Some(manifest) = read_manifest(connection, trace_id, action_id)? else {
        return Ok(None);
    };
    let refs = read_refs(connection, manifest.manifest_id)?;
    let blocks = read_blocks(connection, &refs)?;
    let skeleton = serde_json::from_str::<Value>(&manifest.skeleton_json).map_err(|error| {
        SemanticActionStoreError::new("parse_llm_request_skeleton", error.to_string())
    })?;
    let hydrated = hydrate_value(skeleton, &blocks)?;
    let body_json = canonical_json_string(&hydrated);
    if body_json.len() as u64 != manifest.canonical_body_bytes {
        return Err(SemanticActionStoreError::new(
            "llm_request_body_size_mismatch",
            "reconstructed request body size does not match manifest",
        ));
    }
    let body_hash = sha256_digest_text(body_json.as_bytes());
    if body_hash != manifest.canonical_body_hash {
        return Err(SemanticActionStoreError::new(
            "llm_request_body_hash_mismatch",
            "reconstructed request body hash does not match manifest",
        ));
    }
    let truncated = body_json.len() > max_bytes;
    let body_json = if truncated {
        utf8_prefix(&body_json, max_bytes)
    } else {
        body_json
    };
    Ok(Some(LlmRequestContentPage {
        trace_id,
        action_id: action_id.to_string(),
        format_version: manifest.format_version,
        canonical_body_hash: manifest.canonical_body_hash,
        canonical_body_bytes: manifest.canonical_body_bytes,
        returned_bytes: body_json.len() as u64,
        truncated,
        body_json,
    }))
}

struct ManifestRow {
    manifest_id: i64,
    format_version: u32,
    canonical_body_hash: String,
    canonical_body_bytes: u64,
    skeleton_json: String,
}

struct RefRow {
    ordinal: u32,
    block_id: i64,
}

fn read_manifest(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    action_id: &str,
) -> Result<Option<ManifestRow>, SemanticActionStoreError> {
    connection
        .query_row(
            "SELECT manifest_id, format_version, canonical_body_hash,
                    canonical_body_bytes, skeleton_json
             FROM llm_request_manifests
             WHERE trace_id = ?1 AND action_id = ?2",
            params![trace_id.get(), action_id],
            |row| {
                let hash = row.get::<_, Vec<u8>>("canonical_body_hash")?;
                Ok(ManifestRow {
                    manifest_id: row.get("manifest_id")?,
                    format_version: row.get("format_version")?,
                    canonical_body_hash: sha256_hash_text(&hash),
                    canonical_body_bytes: row.get("canonical_body_bytes")?,
                    skeleton_json: row.get("skeleton_json")?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_llm_request_manifest", error.to_string())
        })
}

fn read_refs(
    connection: &rusqlite::Connection,
    manifest_id: i64,
) -> Result<Vec<RefRow>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT ordinal, block_id
             FROM llm_request_block_refs
             WHERE manifest_id = ?1
             ORDER BY ordinal ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new("prepare_llm_request_refs", error.to_string())
        })?;
    let rows = statement
        .query_map(params![manifest_id], |row| {
            Ok(RefRow {
                ordinal: row.get("ordinal")?,
                block_id: row.get("block_id")?,
            })
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_llm_request_refs", error.to_string())
        })?;
    rows.map(|row| {
        row.map_err(|error| {
            SemanticActionStoreError::new("map_llm_request_refs", error.to_string())
        })
    })
    .collect()
}

fn read_blocks(
    connection: &rusqlite::Connection,
    refs: &[RefRow],
) -> Result<BTreeMap<u32, Value>, SemanticActionStoreError> {
    let mut blocks = BTreeMap::new();
    for block_ref in refs {
        let encoded_bytes = connection
            .query_row(
                "SELECT encoded_bytes
                 FROM llm_request_blocks
                 WHERE block_id = ?1",
                params![block_ref.block_id],
                |row| row.get::<_, Vec<u8>>("encoded_bytes"),
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_llm_request_block", error.to_string())
            })?
            .ok_or_else(|| {
                SemanticActionStoreError::new(
                    "llm_request_block_missing",
                    format!("block_id {} is missing", block_ref.block_id),
                )
            })?;
        let value = serde_json::from_slice::<Value>(&encoded_bytes).map_err(|error| {
            SemanticActionStoreError::new("parse_llm_request_block", error.to_string())
        })?;
        blocks.insert(block_ref.ordinal, value);
    }
    Ok(blocks)
}

fn hydrate_value(
    value: Value,
    blocks: &BTreeMap<u32, Value>,
) -> Result<Value, SemanticActionStoreError> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(|value| hydrate_value(value, blocks))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(object) => {
            if let Some(ordinal) = block_placeholder_ordinal(&object) {
                return blocks.get(&ordinal).cloned().ok_or_else(|| {
                    SemanticActionStoreError::new(
                        "llm_request_block_ref_missing",
                        format!("skeleton references missing block ordinal {ordinal}"),
                    )
                });
            }
            object
                .into_iter()
                .map(|(key, value)| hydrate_value(value, blocks).map(|value| (key, value)))
                .collect::<Result<Map<String, Value>, _>>()
                .map(Value::Object)
        }
        other => Ok(other),
    }
}

fn block_placeholder_ordinal(object: &Map<String, Value>) -> Option<u32> {
    if object.len() != 1 {
        return None;
    }
    let value = object.get(BLOCK_PLACEHOLDER_KEY)?;
    u32::try_from(value.as_u64()?).ok()
}

fn canonical_json_string(value: &Value) -> String {
    let mut output = String::new();
    write_canonical_json(&mut output, value);
    output
}

fn write_canonical_json(output: &mut String, value: &Value) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output
            .push_str(&serde_json::to_string(value).expect("serializing JSON string cannot fail")),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(output, value);
            }
            output.push(']');
        }
        Value::Object(object) => {
            output.push('{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("serializing JSON key cannot fail"),
                );
                output.push(':');
                let value = object.get(key).expect("object key came from object");
                write_canonical_json(output, value);
            }
            output.push('}');
        }
    }
}

fn sha256_hash_text(bytes: &[u8]) -> String {
    let mut output = String::from("sha256:");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to string cannot fail");
    }
    output
}

fn sha256_digest_text(bytes: &[u8]) -> String {
    sha256_hash_text(&Sha256::digest(bytes))
}

fn utf8_prefix(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

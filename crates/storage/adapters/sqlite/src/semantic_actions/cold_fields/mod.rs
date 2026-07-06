//! Compressed cold-field storage for semantic action payload-like values.

use rusqlite::{Row, params};
use sha2::{Digest, Sha256};

use crate::semantic_actions::storage_meta::{ColdFieldMeta, current};

pub(in crate::semantic_actions) struct EncodedColdField {
    pub encoding_code: i16,
    pub uncompressed_bytes: i64,
    pub value_hash: Vec<u8>,
    pub payload: Vec<u8>,
}

pub(in crate::semantic_actions) fn encode_text(
    value: &str,
) -> Result<EncodedColdField, rusqlite::Error> {
    let meta = current().cold_fields;
    let raw = value.as_bytes();
    let (encoding_code, payload) = if raw.len() >= meta.compression_min_bytes {
        let compressed = zstd::stream::encode_all(raw, meta.zstd_level)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        if compressed.len() < raw.len() {
            (meta.zstd, compressed)
        } else {
            (meta.plain_text, raw.to_vec())
        }
    } else {
        (meta.plain_text, raw.to_vec())
    };
    Ok(EncodedColdField {
        encoding_code,
        uncompressed_bytes: i64::try_from(raw.len()).map_err(|_| rusqlite::Error::InvalidQuery)?,
        value_hash: sha256_hash(raw),
        payload,
    })
}

pub(in crate::semantic_actions) fn decode_text_from_row(
    row: &Row<'_>,
    legacy_column: &str,
) -> Result<String, rusqlite::Error> {
    decode_text_from_row_with_prefix(row, legacy_column, "attributes")
}

pub(in crate::semantic_actions) fn decode_text_from_row_with_prefix(
    row: &Row<'_>,
    legacy_column: &str,
    prefix: &str,
) -> Result<String, rusqlite::Error> {
    let encoding_column = format!("{prefix}_encoding_code");
    let Some(encoding_code) = row.get::<_, Option<i64>>(encoding_column.as_str())? else {
        return row.get(legacy_column);
    };
    let uncompressed_bytes = row.get::<_, i64>(format!("{prefix}_uncompressed_bytes").as_str())?;
    let value_hash = row.get::<_, Vec<u8>>(format!("{prefix}_value_hash").as_str())?;
    let payload = row.get::<_, Vec<u8>>(format!("{prefix}_payload").as_str())?;
    let meta = current().cold_fields;
    let bytes = decode_payload(&payload, encoding_code, uncompressed_bytes, meta)?;
    if i64::try_from(bytes.len()).map_err(|_| rusqlite::Error::InvalidQuery)? != uncompressed_bytes
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if sha256_hash(&bytes) != value_hash {
        return Err(rusqlite::Error::InvalidQuery);
    }
    String::from_utf8(bytes).map_err(|_| rusqlite::Error::InvalidQuery)
}

pub(in crate::semantic_actions) fn upsert_action_attributes(
    connection: &rusqlite::Connection,
    action_key: i64,
    value: &str,
) -> Result<(), rusqlite::Error> {
    let field_code = current().cold_fields.action_attributes;
    if value.is_empty() {
        return delete_action_field(connection, action_key, field_code);
    }
    let encoded = encode_text(value)?;
    connection.execute(
        "INSERT OR REPLACE INTO semantic_action_cold_fields (
            owner_key, field_code, encoding_code, uncompressed_bytes, value_hash, payload
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            action_key,
            field_code,
            encoded.encoding_code,
            encoded.uncompressed_bytes,
            encoded.value_hash,
            encoded.payload,
        ],
    )?;
    Ok(())
}

pub(in crate::semantic_actions) fn upsert_link_attributes(
    connection: &rusqlite::Connection,
    trace_id: u64,
    parent_action_key: i64,
    child_action_key: i64,
    role_code: i16,
    value: &str,
) -> Result<(), rusqlite::Error> {
    let field_code = current().cold_fields.link_attributes;
    if value.is_empty() {
        return delete_link_field(
            connection,
            trace_id,
            parent_action_key,
            child_action_key,
            role_code,
            field_code,
        );
    }
    let encoded = encode_text(value)?;
    connection.execute(
        "INSERT OR REPLACE INTO semantic_action_link_cold_fields (
            trace_id, parent_action_key, child_action_key, role_code, field_code,
            encoding_code, uncompressed_bytes, value_hash, payload
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            trace_id,
            parent_action_key,
            child_action_key,
            role_code,
            field_code,
            encoded.encoding_code,
            encoded.uncompressed_bytes,
            encoded.value_hash,
            encoded.payload,
        ],
    )?;
    Ok(())
}

fn delete_action_field(
    connection: &rusqlite::Connection,
    action_key: i64,
    field_code: i16,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "DELETE FROM semantic_action_cold_fields
         WHERE owner_key = ?1 AND field_code = ?2",
        params![action_key, field_code],
    )?;
    Ok(())
}

fn delete_link_field(
    connection: &rusqlite::Connection,
    trace_id: u64,
    parent_action_key: i64,
    child_action_key: i64,
    role_code: i16,
    field_code: i16,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "DELETE FROM semantic_action_link_cold_fields
         WHERE trace_id = ?1
           AND parent_action_key = ?2
           AND child_action_key = ?3
           AND role_code = ?4
           AND field_code = ?5",
        params![
            trace_id,
            parent_action_key,
            child_action_key,
            role_code,
            field_code,
        ],
    )?;
    Ok(())
}

fn decode_payload(
    payload: &[u8],
    encoding_code: i64,
    uncompressed_bytes: i64,
    meta: ColdFieldMeta,
) -> Result<Vec<u8>, rusqlite::Error> {
    if encoding_code == i64::from(meta.plain_text) {
        return Ok(payload.to_vec());
    }
    if encoding_code == i64::from(meta.zstd) {
        let limit =
            usize::try_from(uncompressed_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
        return zstd::stream::decode_all(std::io::Cursor::new(payload))
            .map(|bytes| bytes.into_iter().take(limit.saturating_add(1)).collect())
            .map_err(|_| rusqlite::Error::InvalidQuery);
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn sha256_hash(input: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(input);
    hasher.finalize().to_vec()
}

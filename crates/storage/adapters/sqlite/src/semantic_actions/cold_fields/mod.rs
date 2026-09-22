//! Compressed cold-field storage for semantic action payload-like values.

use std::collections::BTreeMap;

use rusqlite::{Row, params};

use crate::semantic_actions::attribute_codes::decode_attributes;
use crate::semantic_actions::storage_meta::{ColdFieldMeta, current};

mod encoder;
pub(crate) use encoder::ColdFieldEncoder;

pub(in crate::semantic_actions) struct EncodedColdField {
    pub encoding_code: i16,
    pub uncompressed_bytes: i64,
    pub payload: Vec<u8>,
}

pub(in crate::semantic_actions) fn decode_attributes_from_row(
    row: &Row<'_>,
) -> Result<BTreeMap<String, String>, rusqlite::Error> {
    decode_attributes_from_row_with_prefix(row, "attributes")
}

pub(in crate::semantic_actions) fn decode_attributes_from_row_with_prefix(
    row: &Row<'_>,
    prefix: &str,
) -> Result<BTreeMap<String, String>, rusqlite::Error> {
    let encoding_column = format!("{prefix}_encoding_code");
    let Some(encoding_code) = row.get::<_, Option<i64>>(encoding_column.as_str())? else {
        return Ok(BTreeMap::new());
    };
    let uncompressed_bytes = row.get::<_, i64>(format!("{prefix}_uncompressed_bytes").as_str())?;
    let payload = row.get::<_, Vec<u8>>(format!("{prefix}_payload").as_str())?;
    let meta = current().cold_fields;
    let bytes = decode_payload(&payload, encoding_code, uncompressed_bytes, meta)?;
    if i64::try_from(bytes.len()).map_err(|_| rusqlite::Error::InvalidQuery)? != uncompressed_bytes
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    decode_attributes(&bytes)
}

pub(in crate::semantic_actions) fn upsert_action_attributes(
    connection: &mut rusqlite::Connection,
    action_key: i64,
    attributes: &BTreeMap<String, String>,
    encoder: &ColdFieldEncoder,
) -> Result<(), rusqlite::Error> {
    if attributes.is_empty() {
        return Ok(());
    }
    let encoded = encoder.encode_compact(attributes)?;
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO semantic_action_cold_fields (
            owner_key, encoding_code, uncompressed_bytes, payload
         ) VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![
            action_key,
            encoded.encoding_code,
            encoded.uncompressed_bytes,
            encoded.payload,
        ])?;
    Ok(())
}

pub(in crate::semantic_actions) fn upsert_link_attributes(
    connection: &mut rusqlite::Connection,
    trace_id: u64,
    parent_action_key: i64,
    child_action_key: i64,
    role_code: i16,
    attributes: &BTreeMap<String, String>,
    encoder: &ColdFieldEncoder,
) -> Result<(), rusqlite::Error> {
    if attributes.is_empty() {
        return delete_link_field(
            connection,
            trace_id,
            parent_action_key,
            child_action_key,
            role_code,
        );
    }
    let encoded = encoder.encode_compact(attributes)?;
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO semantic_action_link_cold_fields (
            trace_id, parent_action_key, child_action_key, role_code,
            encoding_code, uncompressed_bytes, payload
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
            trace_id,
            parent_action_key,
            child_action_key,
            role_code,
            encoded.encoding_code,
            encoded.uncompressed_bytes,
            encoded.payload,
        ])?;
    Ok(())
}

fn delete_link_field(
    connection: &mut rusqlite::Connection,
    trace_id: u64,
    parent_action_key: i64,
    child_action_key: i64,
    role_code: i16,
) -> Result<(), rusqlite::Error> {
    connection
        .prepare_cached(
            "DELETE FROM semantic_action_link_cold_fields
         WHERE trace_id = ?1
           AND parent_action_key = ?2
           AND child_action_key = ?3
           AND role_code = ?4",
        )?
        .execute(params![
            trace_id,
            parent_action_key,
            child_action_key,
            role_code,
        ])?;
    Ok(())
}

fn decode_payload(
    payload: &[u8],
    encoding_code: i64,
    uncompressed_bytes: i64,
    meta: ColdFieldMeta,
) -> Result<Vec<u8>, rusqlite::Error> {
    if encoding_code == i64::from(meta.plain_text) || encoding_code == i64::from(meta.compact_plain)
    {
        return Ok(payload.to_vec());
    }
    if encoding_code == i64::from(meta.zstd) || encoding_code == i64::from(meta.compact_zstd) {
        let limit =
            usize::try_from(uncompressed_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
        return zstd::stream::decode_all(std::io::Cursor::new(payload))
            .map(|bytes| bytes.into_iter().take(limit.saturating_add(1)).collect())
            .map_err(|_| rusqlite::Error::InvalidQuery);
    }
    Err(rusqlite::Error::InvalidQuery)
}

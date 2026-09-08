//! Compact owner-local encoding for ordered semantic evidence.

use semantic_action::SemanticEvidence;

use crate::semantic_actions::codebook::evidence_role;
use crate::semantic_actions::codebook::sqlite::{decode_evidence_kind, evidence_kind_code};

const FORMAT_VERSION: u8 = 1;

pub(super) fn encode(evidence: &[SemanticEvidence]) -> Result<Vec<u8>, rusqlite::Error> {
    let mut bytes = Vec::with_capacity(2usize.saturating_add(evidence.len().saturating_mul(5)));
    bytes.push(FORMAT_VERSION);
    write_varint(&mut bytes, evidence.len() as u64);
    for item in evidence {
        write_varint(&mut bytes, evidence_kind_code(item.kind) as u64);
        write_varint(&mut bytes, item.id);
        let role = evidence_role::encode(&item.role);
        write_varint(&mut bytes, role.code as u64);
        if let Some(inline) = role.inline {
            write_varint(&mut bytes, inline.len() as u64);
            bytes.extend_from_slice(inline.as_bytes());
        }
    }
    Ok(bytes)
}

pub(super) fn decode(bytes: &[u8]) -> Result<Vec<SemanticEvidence>, rusqlite::Error> {
    let mut cursor = 0usize;
    if read_u8(bytes, &mut cursor)? != FORMAT_VERSION {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let count = read_usize(bytes, &mut cursor)?;
    if count > bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut evidence = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = decode_evidence_kind(read_i64(bytes, &mut cursor)?)?;
        let id = read_varint(bytes, &mut cursor)?;
        let role_code = read_i64(bytes, &mut cursor)?;
        let inline = if role_code == 0 {
            Some(read_string(bytes, &mut cursor)?)
        } else {
            None
        };
        evidence.push(SemanticEvidence {
            kind,
            id,
            role: evidence_role::decode(role_code, inline)?,
        });
    }
    if cursor != bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(evidence)
}

fn write_varint(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            bytes.push(byte);
            return;
        }
        bytes.push(byte | 0x80);
    }
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, rusqlite::Error> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = read_u8(bytes, cursor)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= u64::BITS {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, rusqlite::Error> {
    let byte = bytes
        .get(*cursor)
        .copied()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    *cursor += 1;
    Ok(byte)
}

fn read_usize(bytes: &[u8], cursor: &mut usize) -> Result<usize, rusqlite::Error> {
    usize::try_from(read_varint(bytes, cursor)?).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn read_i64(bytes: &[u8], cursor: &mut usize) -> Result<i64, rusqlite::Error> {
    i64::try_from(read_varint(bytes, cursor)?).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn read_string(bytes: &[u8], cursor: &mut usize) -> Result<String, rusqlite::Error> {
    let length = read_usize(bytes, cursor)?;
    let end = cursor
        .checked_add(length)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    *cursor = end;
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| rusqlite::Error::InvalidQuery)
}

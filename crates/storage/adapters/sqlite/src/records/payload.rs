//! Event-payload encoding used by the SQLite storage adapter.
//!
//! Large fields are split out first (`event_codec::split_large_fields`), then
//! the remaining small fields serialize through the swappable codec. See
//! `records::event_codec` for the codec seam and block split.

use model_core::event::EventPayload;
use rusqlite::Error as SqlError;

use super::event_codec::{
    EncodedEventPayload, PayloadBlock, event_payload_codec, join_large_fields, split_large_fields,
};

pub fn encode_event_payload(payload: &mut EventPayload) -> Result<EncodedEventPayload, SqlError> {
    let blocks = split_large_fields(payload);
    let fields = event_payload_codec()
        .encode(payload)
        .map_err(|_| SqlError::InvalidQuery)?;
    Ok(EncodedEventPayload { fields, blocks })
}

pub fn decode_event_payload(
    fields: &[u8],
    blocks: &[PayloadBlock],
) -> Result<EventPayload, SqlError> {
    let mut payload = event_payload_codec()
        .decode(fields)
        .map_err(|_| SqlError::InvalidQuery)?;
    if !payload_blocks_match(&payload, blocks) {
        return Err(SqlError::InvalidQuery);
    }
    join_large_fields(&mut payload, blocks);
    Ok(payload)
}

fn payload_blocks_match(payload: &EventPayload, blocks: &[PayloadBlock]) -> bool {
    if blocks.is_empty() {
        return true;
    }
    if blocks.len() != 1 {
        return false;
    }
    match (payload, blocks[0].kind) {
        (EventPayload::Stdio(_), super::BlockKind::StdioData) => true,
        (
            EventPayload::Application(_),
            super::BlockKind::HttpBodyText
            | super::BlockKind::HttpBodyJson
            | super::BlockKind::HttpBodyBase64,
        ) => true,
        _ => false,
    }
}

pub(crate) fn take_shared_path(payload: &mut EventPayload) -> Option<String> {
    match payload {
        EventPayload::Process(payload) => payload.executable.take(),
        EventPayload::File(payload) => payload.path.take(),
        EventPayload::Enforcement(payload) => payload.path.take(),
        _ => None,
    }
}

pub(crate) fn restore_shared_path(
    payload: &mut EventPayload,
    path: Option<String>,
) -> Result<(), SqlError> {
    let target = match payload {
        EventPayload::Process(payload) => &mut payload.executable,
        EventPayload::File(payload) => &mut payload.path,
        EventPayload::Enforcement(payload) => &mut payload.path,
        _ if path.is_none() => return Ok(()),
        _ => return Err(SqlError::InvalidQuery),
    };
    if target.is_some() && path.is_some() {
        return Err(SqlError::InvalidQuery);
    }
    if path.is_some() {
        *target = path;
    }
    Ok(())
}

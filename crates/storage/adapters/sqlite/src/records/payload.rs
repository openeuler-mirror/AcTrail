//! Event-payload encoding used by the SQLite storage adapter.
//!
//! Large fields are split out first (`event_codec::split_large_fields`), then
//! the remaining small fields serialize through the swappable codec. See
//! `records::event_codec` for the codec seam and block split.

use model_core::event::EventPayload;
use rusqlite::Error as SqlError;

use super::event_codec::{
    EncodedEventPayload, PayloadBlock, event_payload_codec, join_large_fields, split_large_fields,
    variant_str,
};

pub fn encode_event_payload(payload: &mut EventPayload) -> Result<EncodedEventPayload, SqlError> {
    let blocks = split_large_fields(payload);
    let variant = variant_str(payload);
    let fields = event_payload_codec()
        .encode(payload)
        .map_err(|_| SqlError::InvalidQuery)?;
    Ok(EncodedEventPayload {
        variant,
        fields,
        blocks,
    })
}

pub fn decode_event_payload(
    fields: &[u8],
    blocks: &[PayloadBlock],
) -> Result<EventPayload, SqlError> {
    let mut payload = event_payload_codec()
        .decode(fields)
        .map_err(|_| SqlError::InvalidQuery)?;
    join_large_fields(&mut payload, blocks);
    Ok(payload)
}

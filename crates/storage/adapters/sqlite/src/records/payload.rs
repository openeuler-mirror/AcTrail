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

#[cfg(test)]
mod tests {
    use model_core::event::{
        EventPayload, ResourceAccountingCoverage, ResourceAccountingMethod, ResourceSampleKind,
    };
    use rusqlite::{Connection, params};

    use crate::records::event_from_row;

    // A tag-6 resource payload written before ResourceV2 was introduced.
    const LEGACY_RESOURCE_FIXTURE: &[u8] = &[
        6, 7, b'p', b'r', b'o', b'c', b'e', b's', b's', 6, b'p', b'i', b'd', b':', b'4', b'2', 1,
        0xe2, 0x04, 0, 0, 0, 0, 0, 0, 1, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xff, 10, b'f', b'u',
        b't', b'u', b'r', b'e', b'_', b'k', b'e', b'y', 4, b'k', b'e', b'p', b't',
    ];

    #[test]
    fn sqlite_row_with_pre_change_resource_payload_remains_readable() {
        let connection = Connection::open_in_memory().unwrap();
        let event = connection
            .query_row(
                "SELECT
                    1 AS event_id,
                    2 AS trace_id,
                    0 AS observed_at,
                    3 AS process_id,
                    4 AS event_meta,
                    6 AS kind_code,
                    NULL AS payload_path_id,
                    ?1 AS payload,
                    NULL AS payload_path",
                params![LEGACY_RESOURCE_FIXTURE],
                |row| event_from_row(&connection, row),
            )
            .unwrap();
        let EventPayload::Resource(payload) = event.payload else {
            panic!("expected resource payload");
        };
        assert_eq!(
            payload.accounting_method,
            ResourceAccountingMethod::ProcfsRssSum
        );
        assert_eq!(
            payload.accounting_coverage,
            ResourceAccountingCoverage::Partial
        );
        assert_eq!(payload.sample_kind, ResourceSampleKind::Periodic);
        assert_eq!(payload.rss_kb, Some(64));
    }
}

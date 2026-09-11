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
        connection
            .execute_batch(
                "CREATE TABLE events (
                    event_id INTEGER NOT NULL,
                    trace_id INTEGER NOT NULL,
                    observed_at INTEGER NOT NULL,
                    process_id INTEGER NOT NULL,
                    collector TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    bootstrap_observed INTEGER NOT NULL,
                    metadata_partial INTEGER NOT NULL,
                    policy_modified INTEGER NOT NULL,
                    payload BLOB NOT NULL,
                    payload_blocks TEXT NOT NULL,
                    policy_verdict TEXT NOT NULL,
                    policy_note TEXT,
                    policy_redactions TEXT NOT NULL,
                    policy_truncations TEXT NOT NULL
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO events VALUES (
                    1, 2, 0, 3, 'resource-metrics', 'resource', 0, 0, 0,
                    ?1, '', 'allow', NULL, '', ''
                )",
                params![LEGACY_RESOURCE_FIXTURE],
            )
            .unwrap();

        let event = connection
            .query_row("SELECT * FROM events", [], |row| {
                event_from_row(&connection, row)
            })
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

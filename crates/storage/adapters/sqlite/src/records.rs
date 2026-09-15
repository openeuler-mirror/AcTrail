//! Storage-record mapping between core models and SQLite rows.

mod enums;
mod event_codec;
mod event_codes;
mod event_record_blocks;
mod helpers;
mod path_dictionary;
mod payload;
mod payload_dictionary;
mod payload_segment_meta;
mod policy;
mod rows;

pub use enums::{
    decode_diagnostic_kind, decode_diagnostic_severity, decode_exit_observation_source,
    decode_membership_state, decode_trace_health, decode_trace_lifecycle, encode_diagnostic_kind,
    encode_diagnostic_severity, encode_exit_observation_source, encode_membership_state,
    encode_trace_health, encode_trace_lifecycle,
};
pub use event_codec::{BlockKind, EncodedEventPayload, PayloadBlock};
pub use event_codes::{
    EventMeta, decode_event_kind, encode_event_kind, event_kind_name, payload_kind,
};
pub(crate) use event_record_blocks::{
    EventRecordBlockWriter, decode_block as decode_event_record_block,
    decode_event as decode_event_record_frame,
    decode_kind_counts as decode_event_record_block_kind_counts,
};
pub(crate) use helpers::escape;
pub use helpers::{
    bool_to_i64, decode_map, decode_tags, decode_time, encode_map, encode_tags, encode_time,
    i64_to_bool,
};
pub(crate) use path_dictionary::PathInterner;
pub use payload::{decode_event_payload, encode_event_payload};
pub(crate) use payload::{restore_shared_path, take_shared_path};
pub(crate) use payload_dictionary::{EventPayloadDictionary, StoredEventPayload};
pub(crate) use payload_segment_meta::{PAYLOAD_DIRECTION_MASK, PayloadSegmentMeta};
pub use policy::{decode_policy_record, encode_policy_record};
pub use rows::{
    diagnostic_from_row, event_from_row, membership_from_row, payload_segment_from_row,
    trace_from_row,
};

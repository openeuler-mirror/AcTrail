//! Storage-record mapping between core models and SQLite rows.

mod enums;
mod helpers;
mod payload;
mod policy;
mod rows;

pub use enums::{
    decode_diagnostic_kind, decode_diagnostic_severity, decode_event_kind,
    decode_exit_observation_source, decode_membership_state, decode_payload_content_state,
    decode_payload_direction, decode_payload_operation_completion_state,
    decode_payload_redaction_state, decode_payload_source_boundary,
    decode_payload_truncation_state, decode_trace_health, decode_trace_lifecycle,
    encode_diagnostic_kind, encode_diagnostic_severity, encode_event_kind,
    encode_exit_observation_source, encode_membership_state, encode_payload_content_state,
    encode_payload_direction, encode_payload_operation_completion_state,
    encode_payload_redaction_state, encode_payload_source_boundary,
    encode_payload_truncation_state, encode_policy_verdict, encode_trace_health,
    encode_trace_lifecycle,
};
pub use helpers::{
    bool_to_i64, decode_map, decode_tags, decode_time, encode_map, encode_tags, encode_time,
    i64_to_bool,
};
pub use payload::{decode_event_payload, encode_event_payload, encode_process_identity_inline};
pub use policy::{decode_policy_record, encode_policy_record};
pub use rows::{
    diagnostic_from_row, event_from_row, membership_from_row, payload_segment_from_row,
    trace_from_row,
};

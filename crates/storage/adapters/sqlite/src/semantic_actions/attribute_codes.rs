//! Attribute key dictionary and compact cold-field payload encoding.
//!
//! Replaces the `k=v\n` escaped text layout with a compact binary layout where
//! statically known attribute keys are stored as 2-byte codes. Keys outside the
//! dictionary fall back to the escaped text form so no attribute is ever dropped.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::records::{decode_map, escape};

/// Statically known attribute keys, assigned 1-based codes in declaration order.
/// This list mirrors `crates/contracts/semantic_action/src/attr_keys.rs`; keys
/// missing here simply fall back to the text encoding without data loss.
const KNOWN_KEYS: &[&str] = &[
    "actrail.action.finalized_on_trace_close",
    "actrail.action.valid",
    "actrail.link.source",
    "actrail.link.valid",
    "agent.child.command_line",
    "agent.child.executable",
    "agent.child.process_id",
    "agent.exit.identity_action_id",
    "agent.identity.evidence_action_id",
    "agent.identity.source",
    "agent.identity.status",
    "agent.invocation.evidence_action_id",
    "agent.invocation.trigger",
    "agent.performed_action.sequence",
    "agent.turn.user_input_observed_at_unix_nanos",
    "agent.turn.user_input_segment_id",
    "agent.turn.user_input_source",
    "command.exit_code",
    "command.failure.kind",
    "command.failure.summary",
    "command.line",
    "command.tool.name",
    "enforcement.backend",
    "enforcement.decision",
    "enforcement.operation",
    "enforcement.result",
    "enforcement.rule_id",
    "file.bulk_read.chunking_scheme",
    "file.bulk_read.close_count",
    "file.bulk_read.error_count",
    "file.bulk_read.error_path_overflow",
    "file.bulk_read.error_reason_counts",
    "file.bulk_read.error_stored_path_count",
    "file.bulk_read.error_unique_path_count",
    "file.bulk_read.error_unique_path_count_state",
    "file.bulk_read.first_event_id",
    "file.bulk_read.last_event_id",
    "file.bulk_read.mode",
    "file.bulk_read.open_count",
    "file.bulk_read.path_overflow",
    "file.bulk_read.path_set_id",
    "file.bulk_read.path_set_state",
    "file.bulk_read.read_count",
    "file.bulk_read.stored_path_count",
    "file.bulk_read.unique_path_count",
    "file.bulk_read.unique_path_count_state",
    "file.bytes_read",
    "file.bytes_written",
    "file.change_kind",
    "file.error_count",
    "file.fd",
    "file.operation",
    "file.path",
    "file.read_count",
    "file.tty",
    "file.tty.close_count",
    "file.tty.error_count",
    "file.tty.event_count",
    "file.tty.first_event_id",
    "file.tty.last_event_id",
    "file.tty.open_count",
    "file.tty.read_count",
    "file.tty.write_count",
    "file.write_count",
    "fs.enumerate.chunking_scheme",
    "fs.enumerate.close_count",
    "fs.enumerate.error_count",
    "fs.enumerate.error_path_overflow",
    "fs.enumerate.error_reason_counts",
    "fs.enumerate.error_stored_path_count",
    "fs.enumerate.error_unique_path_count",
    "fs.enumerate.error_unique_path_count_state",
    "fs.enumerate.first_event_id",
    "fs.enumerate.last_event_id",
    "fs.enumerate.open_count",
    "fs.enumerate.path_overflow",
    "fs.enumerate.path_set_id",
    "fs.enumerate.path_set_state",
    "fs.enumerate.stored_path_count",
    "fs.enumerate.unique_path_count",
    "fs.enumerate.unique_path_count_state",
    "http.operation",
    "http.request.action_id",
    "http.request.body_contains_nul",
    "http.request.body_json",
    "http.request.body_json_state",
    "http.request.body_text",
    "http.request.headers_encoding",
    "http.request.headers_hpack_base64",
    "http.request.headers_text",
    "http.request.method",
    "http.request.protocol",
    "http.request.stream_id",
    "http.response.body_format",
    "http.response.body_json",
    "http.response.body_json_state",
    "http.response.body_text",
    "http.response.headers_encoding",
    "http.response.headers_hpack_base64",
    "http.response.headers_text",
    "http.response.protocol",
    "http.response.reason",
    "http.response.status_code",
    "http.response.stream_id",
    "invocation.kind",
    "llm.call.http_response_action_id",
    "llm.call.model",
    "llm.call.request_action_id",
    "llm.call.response_action_id",
    "llm.request.background_kind",
    "llm.request.block_count",
    "llm.request.body_json",
    "llm.request.body_text",
    "llm.request.canonical_body_bytes",
    "llm.request.canonical_body_hash",
    "llm.request.classifier_id",
    "llm.request.content_format_version",
    "llm.request.content_state",
    "llm.request.latest_user_message_hash",
    "llm.request.message_preview",
    "llm.request.model",
    "llm.request.payload_bytes",
    "llm.request.payload_text",
    "llm.request.protocol_id",
    "llm.request.raw_payload_bytes",
    "llm.request.user_message_count",
    "llm.response.action_id",
    "llm.response.body_format",
    "llm.response.cached_prompt_tokens",
    "llm.response.chunk_count",
    "llm.response.completion_tokens",
    "llm.response.content_text",
    "llm.response.done",
    "llm.response.finish_reason",
    "llm.response.model",
    "llm.response.output_text",
    "llm.response.payload_bytes",
    "llm.response.payload_text",
    "llm.response.prompt_cache_hit_tokens",
    "llm.response.prompt_cache_miss_tokens",
    "llm.response.prompt_tokens",
    "llm.response.provider_id",
    "llm.response.raw_payload_bytes",
    "llm.response.reasoning_text",
    "llm.response.reasoning_tokens",
    "llm.response.sse_events_json",
    "llm.response.stream",
    "llm.response.tool_calls_json",
    "llm.response.total_tokens",
    "llm.tool_call.id",
    "llm.tool_call.name",
    "mcp.client.process_id",
    "mcp.exchange.index",
    "mcp.execution.status",
    "mcp.message.direction",
    "mcp.message.id",
    "mcp.message.method",
    "mcp.message.sequence",
    "mcp.request.action_id",
    "mcp.request.id",
    "mcp.response.action_id",
    "mcp.server.name",
    "mcp.stdin.action_id",
    "mcp.stdout.action_id",
    "mcp.tool_call.action_id",
    "mcp.tool_call.request_id",
    "mcp.tool.id",
    "mcp.tool.name",
    "mcp.transport",
    "network.protocol.name",
    "network.protocol.version",
    "payload.aggregate.first_segment_id",
    "payload.aggregate.last_segment_id",
    "payload.library",
    "payload.operation_id",
    "payload.operation_ids",
    "payload.segment_count",
    "payload.sequence",
    "payload.sequence_end",
    "payload.sequence_start",
    "payload.source_boundary",
    "payload.stream_key",
    "payload.symbol",
    "process.executable",
    "process.exit_code",
    "process.failure.kind",
    "process.failure.summary",
    "process.id",
    "process.operation",
    "process.parent.id",
    "process.parent.identity_state",
    "server.address",
    "sse.content_delta_count",
    "sse.data_json_state",
    "sse.done",
    "sse.event_count",
    "sse.events_json",
    "sse.reasoning_delta_count",
    "sse.stream.action_id",
    "sse.tool_delta_count",
    "syscall.result",
    "url.path",
    "url.scheme",
    "agent.invocation.agent_type",
    "agent.invocation.prompt_hash",
    "agent.invocation.tool_call_action_id",
    "agent.invocation.tool_call_id",
    "agent.invocation.tool_name",
    "llm.tool_call.arguments_bytes",
    "llm.tool_call.arguments_hash",
    "llm.tool_call.ordinal",
    "llm.tool_call.response_action_id",
    "llm.tool_result.binding_state",
    "llm.tool_result.content_bytes",
    "llm.tool_result.content_export_state",
    "llm.tool_result.content_hash",
    "llm.tool_result.content_json",
    "llm.tool_result.id",
    "llm.tool_result.is_error",
    "llm.tool_result.ordinal",
    "llm.tool_result.request_action_id",
];

fn key_code_map() -> &'static BTreeMap<&'static str, u16> {
    static KEY_CODE_MAP: OnceLock<BTreeMap<&'static str, u16>> = OnceLock::new();
    KEY_CODE_MAP.get_or_init(|| {
        KNOWN_KEYS
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let code = u16::try_from(index + 1).expect("attribute key count fits u16");
                (*key, code)
            })
            .collect()
    })
}

pub(in crate::semantic_actions) fn key_code(key: &str) -> Option<u16> {
    key_code_map().get(key).copied()
}

fn key_for_code(code: u16) -> Option<&'static str> {
    KNOWN_KEYS.get(usize::from(code).checked_sub(1)?).copied()
}

/// Compact binary encoding of an attribute map.
///
/// Layout:
/// - `u32` LE known-pair count
/// - `known_count × { u16 LE key_code, u32 LE value_len, value bytes }`
/// - `u32` LE unknown-text length
/// - unknown text (`k=v\n` escaped, reusing the legacy text escape rules)
pub(in crate::semantic_actions) fn encode_attributes(map: &BTreeMap<String, String>) -> Vec<u8> {
    let mut known: Vec<(u16, &str)> = Vec::new();
    let mut unknown: Vec<(&str, &str)> = Vec::new();
    for (key, value) in map {
        match key_code(key) {
            Some(code) => known.push((code, value.as_str())),
            None => unknown.push((key.as_str(), value.as_str())),
        }
    }
    let mut out = Vec::with_capacity(map.len() * 16);
    out.extend_from_slice(
        &u32::try_from(known.len())
            .expect("attribute count fits u32")
            .to_le_bytes(),
    );
    for (code, value) in known {
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(value.len())
                .expect("attribute value length fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(value.as_bytes());
    }
    let unknown_text = encode_unknown(&unknown);
    out.extend_from_slice(
        &u32::try_from(unknown_text.len())
            .expect("unknown attribute text fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(unknown_text.as_bytes());
    out
}

fn encode_unknown(unknown: &[(&str, &str)]) -> String {
    unknown
        .iter()
        .map(|(key, value)| format!("{}={}", escape(key), escape(value)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(in crate::semantic_actions) fn decode_attributes(
    bytes: &[u8],
) -> Result<BTreeMap<String, String>, rusqlite::Error> {
    let mut cursor = 0usize;
    let known_count = usize::try_from(read_u32(bytes, &mut cursor)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let mut map = BTreeMap::new();
    for _ in 0..known_count {
        let code = read_u16(bytes, &mut cursor)?;
        let value_len = usize::try_from(read_u32(bytes, &mut cursor)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let value = std::str::from_utf8(slice(bytes, cursor, value_len)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        cursor += value_len;
        let key = key_for_code(code).ok_or(rusqlite::Error::InvalidQuery)?;
        map.insert(key.to_string(), value.to_string());
    }
    let unknown_len = usize::try_from(read_u32(bytes, &mut cursor)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let unknown_text = std::str::from_utf8(slice(bytes, cursor, unknown_len)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    cursor += unknown_len;
    for (key, value) in decode_map(unknown_text) {
        map.insert(key, value);
    }
    if cursor != bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(map)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, rusqlite::Error> {
    let raw: [u8; 4] = slice(bytes, *cursor, 4)?
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    *cursor += 4;
    Ok(u32::from_le_bytes(raw))
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, rusqlite::Error> {
    let raw: [u8; 2] = slice(bytes, *cursor, 2)?
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    *cursor += 2;
    Ok(u16::from_le_bytes(raw))
}

fn slice(bytes: &[u8], start: usize, len: usize) -> Result<&[u8], rusqlite::Error> {
    bytes
        .get(
            start
                ..start
                    .checked_add(len)
                    .ok_or(rusqlite::Error::InvalidQuery)?,
        )
        .ok_or(rusqlite::Error::InvalidQuery)
}

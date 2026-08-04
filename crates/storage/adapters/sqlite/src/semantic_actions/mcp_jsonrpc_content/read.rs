use model_core::ids::TraceId;
use rusqlite::{OptionalExtension, params};
use semantic_action::{McpJsonRpcContentPage, SemanticActionStoreError};

use crate::semantic_actions::action_ids::resolve_action_key;

use super::canonical::CanonicalJson;

pub(in crate::semantic_actions) fn mcp_jsonrpc_content_page(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
    action_id: &str,
    max_bytes: usize,
) -> Result<Option<McpJsonRpcContentPage>, SemanticActionStoreError> {
    McpJsonRpcContentReader { connection }.read(trace_id, action_id, max_bytes)
}

struct McpJsonRpcContentReader<'a> {
    connection: &'a rusqlite::Connection,
}

impl McpJsonRpcContentReader<'_> {
    fn read(
        &self,
        trace_id: TraceId,
        action_id: &str,
        max_bytes: usize,
    ) -> Result<Option<McpJsonRpcContentPage>, SemanticActionStoreError> {
        let Some(action_key) = resolve_action_key(self.connection, action_id)? else {
            return Ok(None);
        };
        let Some(message_id) = self.read_message_ref(trace_id, action_key)? else {
            return Ok(None);
        };
        self.require_action(trace_id, action_key)?;
        let row = self.read_message(trace_id, message_id)?;
        CanonicalJson::validate_jsonrpc(&row.canonical_json).map_err(|message| {
            SemanticActionStoreError::new("read_mcp_jsonrpc_canonical_json", message)
        })?;
        if row.canonical_json.len() as u64 != row.canonical_json_bytes {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_size_mismatch",
                "stored canonical JSON-RPC byte count does not match its bytes",
            ));
        }
        if CanonicalJson::digest(&row.canonical_json) != row.hash {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_hash_mismatch",
                "stored canonical JSON-RPC SHA-256 does not match its bytes",
            ));
        }
        let canonical_json = String::from_utf8(row.canonical_json).map_err(|error| {
            SemanticActionStoreError::new("mcp_jsonrpc_utf8", error.to_string())
        })?;
        let truncated = canonical_json.len() > max_bytes;
        let canonical_json = if truncated {
            Self::utf8_prefix(&canonical_json, max_bytes).to_string()
        } else {
            canonical_json
        };
        Ok(Some(McpJsonRpcContentPage {
            trace_id,
            action_id: action_id.to_string(),
            format_version: row.format_version,
            canonical_json_hash: CanonicalJson::hash_text(&row.hash),
            canonical_json_bytes: row.canonical_json_bytes,
            returned_bytes: canonical_json.len() as u64,
            truncated,
            canonical_json,
        }))
    }

    fn read_message_ref(
        &self,
        trace_id: TraceId,
        action_key: i64,
    ) -> Result<Option<i64>, SemanticActionStoreError> {
        self.connection
            .query_row(
                "SELECT message_id FROM mcp_jsonrpc_action_refs
                 WHERE trace_id = ?1 AND action_key = ?2",
                params![trace_id.get(), action_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_mcp_jsonrpc_action_ref", error.to_string())
            })
    }

    fn require_action(
        &self,
        trace_id: TraceId,
        action_key: i64,
    ) -> Result<(), SemanticActionStoreError> {
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM semantic_actions WHERE trace_id = ?1 AND action_key = ?2",
                params![trace_id.get(), action_key],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_mcp_jsonrpc_action", error.to_string())
            })?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_action_missing",
                "JSON-RPC content reference points at a missing semantic action",
            ))
        }
    }

    fn read_message(
        &self,
        trace_id: TraceId,
        message_id: i64,
    ) -> Result<MessageRow, SemanticActionStoreError> {
        self.connection
            .query_row(
                "SELECT format_version, canonical_json_hash,
                        canonical_json_bytes, canonical_json
                 FROM mcp_jsonrpc_messages
                 WHERE trace_id = ?1 AND message_id = ?2",
                params![trace_id.get(), message_id],
                |row| {
                    let format_version = u32::try_from(row.get::<_, i64>("format_version")?)
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Integer,
                                Box::new(error),
                            )
                        })?;
                    let canonical_json_bytes = u64::try_from(
                        row.get::<_, i64>("canonical_json_bytes")?,
                    )
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?;
                    Ok(MessageRow {
                        format_version,
                        hash: row.get("canonical_json_hash")?,
                        canonical_json_bytes,
                        canonical_json: row.get("canonical_json")?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_mcp_jsonrpc_message", error.to_string())
            })?
            .ok_or_else(|| {
                SemanticActionStoreError::new(
                    "mcp_jsonrpc_message_missing",
                    "JSON-RPC content reference points at a missing message",
                )
            })
    }

    fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
        let mut end = value.len().min(max_bytes);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        &value[..end]
    }
}

struct MessageRow {
    format_version: u32,
    hash: Vec<u8>,
    canonical_json_bytes: u64,
    canonical_json: Vec<u8>,
}

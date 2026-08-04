use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};
use semantic_action::{McpJsonRpcContentWrite, SemanticActionStoreError};

use crate::semantic_actions::action_ids::require_action_key;

use super::canonical::CanonicalJson;

const BEGIN_CONTENT_BATCH: &str = "SAVEPOINT actrail_mcp_jsonrpc_content_batch";
const COMMIT_CONTENT_BATCH: &str = "RELEASE SAVEPOINT actrail_mcp_jsonrpc_content_batch";
const ROLLBACK_CONTENT_BATCH: &str = "ROLLBACK TO SAVEPOINT actrail_mcp_jsonrpc_content_batch;
RELEASE SAVEPOINT actrail_mcp_jsonrpc_content_batch;";

pub(in crate::semantic_actions) fn upsert_mcp_jsonrpc_contents(
    connection: &rusqlite::Connection,
    contents: &[McpJsonRpcContentWrite],
) -> Result<(), SemanticActionStoreError> {
    McpJsonRpcContentWriter { connection }.upsert_all(contents)
}

struct McpJsonRpcContentWriter<'a> {
    connection: &'a rusqlite::Connection,
}

impl McpJsonRpcContentWriter<'_> {
    fn upsert_all(
        &self,
        contents: &[McpJsonRpcContentWrite],
    ) -> Result<(), SemanticActionStoreError> {
        self.connection
            .execute_batch(BEGIN_CONTENT_BATCH)
            .map_err(|error| {
                SemanticActionStoreError::new("begin_mcp_jsonrpc_content_batch", error.to_string())
            })?;
        let result = self.write_all(contents);
        match result {
            Ok(()) => self
                .connection
                .execute_batch(COMMIT_CONTENT_BATCH)
                .map_err(|error| {
                    SemanticActionStoreError::new(
                        "commit_mcp_jsonrpc_content_batch",
                        error.to_string(),
                    )
                }),
            Err(error) => {
                let rollback = self.connection.execute_batch(ROLLBACK_CONTENT_BATCH);
                match rollback {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(SemanticActionStoreError::new(
                        "rollback_mcp_jsonrpc_content_batch",
                        format!(
                            "{}: {}; rollback failed: {rollback_error}",
                            error.stage, error.message
                        ),
                    )),
                }
            }
        }
    }

    fn write_all(
        &self,
        contents: &[McpJsonRpcContentWrite],
    ) -> Result<(), SemanticActionStoreError> {
        for content in contents {
            self.upsert(content)?;
        }
        Ok(())
    }

    fn upsert(&self, content: &McpJsonRpcContentWrite) -> Result<(), SemanticActionStoreError> {
        self.validate(content)?;
        let hash = CanonicalJson::parse_hash(&content.canonical_json_hash)
            .map_err(|message| SemanticActionStoreError::new("mcp_jsonrpc_hash", message))?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO mcp_jsonrpc_messages (
                    trace_id, format_version, canonical_json_hash,
                    canonical_json_bytes, canonical_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    content.trace_id.get(),
                    i64::from(content.format_version),
                    &hash,
                    self.canonical_len(content)?,
                    &content.canonical_json,
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("insert_mcp_jsonrpc_message", error.to_string())
            })?;
        let message_id = self.verify_message(content, &hash)?;
        for action_id in &content.action_ids {
            self.write_action_ref(content, action_id, message_id)?;
        }
        Ok(())
    }

    fn validate(&self, content: &McpJsonRpcContentWrite) -> Result<(), SemanticActionStoreError> {
        if content.format_version == 0 {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_format_version",
                "format_version must be positive",
            ));
        }
        if content.action_ids.is_empty() {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_action_refs",
                "at least one action_id is required",
            ));
        }
        let mut action_ids = BTreeSet::new();
        for action_id in &content.action_ids {
            if action_id.is_empty() || !action_ids.insert(action_id) {
                return Err(SemanticActionStoreError::new(
                    "mcp_jsonrpc_action_refs",
                    "action_ids must be non-empty and unique",
                ));
            }
        }
        CanonicalJson::validate_jsonrpc(&content.canonical_json).map_err(|message| {
            SemanticActionStoreError::new("mcp_jsonrpc_canonical_json", message)
        })?;
        let expected_hash = CanonicalJson::parse_hash(&content.canonical_json_hash)
            .map_err(|message| SemanticActionStoreError::new("mcp_jsonrpc_hash", message))?;
        if CanonicalJson::digest(&content.canonical_json) != expected_hash {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_hash_mismatch",
                "canonical JSON-RPC SHA-256 does not match its bytes",
            ));
        }
        self.canonical_len(content)?;
        Ok(())
    }

    fn verify_message(
        &self,
        content: &McpJsonRpcContentWrite,
        hash: &[u8],
    ) -> Result<i64, SemanticActionStoreError> {
        let existing = self
            .connection
            .query_row(
                "SELECT message_id, canonical_json_bytes, canonical_json
                 FROM mcp_jsonrpc_messages
                 WHERE trace_id = ?1 AND format_version = ?2 AND canonical_json_hash = ?3",
                params![
                    content.trace_id.get(),
                    i64::from(content.format_version),
                    hash,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>("message_id")?,
                        row.get::<_, i64>("canonical_json_bytes")?,
                        row.get::<_, Vec<u8>>("canonical_json")?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_mcp_jsonrpc_message", error.to_string())
            })?;
        let Some((message_id, bytes, canonical_json)) = existing else {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_message_missing",
                "JSON-RPC message insert did not materialize a row",
            ));
        };
        if bytes != self.canonical_len(content)? || canonical_json != content.canonical_json {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_hash_collision",
                "same trace, format, and SHA-256 identify different canonical JSON-RPC bytes",
            ));
        }
        Ok(message_id)
    }

    fn write_action_ref(
        &self,
        content: &McpJsonRpcContentWrite,
        action_id: &str,
        message_id: i64,
    ) -> Result<(), SemanticActionStoreError> {
        let action_key = require_action_key(self.connection, action_id)?;
        let action_exists = self
            .connection
            .query_row(
                "SELECT 1 FROM semantic_actions WHERE trace_id = ?1 AND action_key = ?2",
                params![content.trace_id.get(), action_key],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_mcp_jsonrpc_action", error.to_string())
            })?
            .is_some();
        if !action_exists {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_action_missing",
                format!("cannot reference missing action {action_id} in the content trace"),
            ));
        }
        let existing = self
            .connection
            .query_row(
                "SELECT message_id FROM mcp_jsonrpc_action_refs
                 WHERE trace_id = ?1 AND action_key = ?2",
                params![content.trace_id.get(), action_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new("read_mcp_jsonrpc_action_ref", error.to_string())
            })?;
        if let Some(existing) = existing {
            return if existing == message_id {
                Ok(())
            } else {
                Err(SemanticActionStoreError::new(
                    "mcp_jsonrpc_action_ref_conflict",
                    format!("action {action_id} already references a different JSON-RPC message"),
                ))
            };
        }
        self.connection
            .execute(
                "INSERT INTO mcp_jsonrpc_action_refs (trace_id, action_key, message_id)
                 VALUES (?1, ?2, ?3)",
                params![content.trace_id.get(), action_key, message_id],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("insert_mcp_jsonrpc_action_ref", error.to_string())
            })?;
        Ok(())
    }

    fn canonical_len(
        &self,
        content: &McpJsonRpcContentWrite,
    ) -> Result<i64, SemanticActionStoreError> {
        i64::try_from(content.canonical_json.len()).map_err(|error| {
            SemanticActionStoreError::new("mcp_jsonrpc_content_size", error.to_string())
        })
    }
}

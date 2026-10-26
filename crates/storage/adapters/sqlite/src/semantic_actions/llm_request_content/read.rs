use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::{Connection, DatabaseName, OptionalExtension, params};
use semantic_action::{ContentNodePage, ContentNodeQuery, SemanticActionStoreError};
use serde_json::Value;

use super::canonical::{CanonicalTraversal, LengthWriter};
use crate::{SqliteStorage, semantic_actions::action_ids::resolve_action_key};

pub(super) struct BlockMeta {
    pub(super) id: i64,
    pub(super) bytes: u64,
}

pub(in crate::semantic_actions) struct ContentReader<'a> {
    pub(super) connection: &'a Connection,
    pub(super) trace_id: TraceId,
    pub(super) action_id: String,
    pub(super) format_version: u32,
    pub(super) skeleton: Value,
    pub(super) blocks: BTreeMap<u32, BlockMeta>,
    pub(super) canonical_body_bytes: u64,
}

impl SqliteStorage {
    pub(crate) fn read_llm_request_content_node(
        &self,
        trace_id: TraceId,
        action_id: &str,
        query: &ContentNodeQuery,
    ) -> Result<Option<ContentNodePage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "llm_request_content_node",
                "trace has been purged",
            ));
        }
        ContentReader::load(&self.connection().borrow(), trace_id, action_id)?
            .map(|reader| reader.node(query))
            .transpose()
    }
}

impl<'a> ContentReader<'a> {
    pub(in crate::semantic_actions) fn load(
        connection: &'a Connection,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Option<Self>, SemanticActionStoreError> {
        let Some(key) = resolve_action_key(connection, action_id)? else {
            return Ok(None);
        };
        let manifest: Option<(i64,u32,String)> = connection.query_row(
            "SELECT manifest_id,format_version,skeleton_json FROM llm_request_manifests WHERE trace_id=?1 AND action_key=?2",
            params![trace_id.get(),key], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
            .optional().map_err(Self::error)?;
        let Some((id, format_version, skeleton)) = manifest else {
            return Ok(None);
        };
        let skeleton = serde_json::from_str(&skeleton).map_err(Self::error)?;
        let mut statement = connection.prepare(
            "SELECT refs.ordinal,blocks.block_id,blocks.trace_id,blocks.uncompressed_bytes,length(blocks.encoded_bytes)
             FROM llm_request_block_refs refs LEFT JOIN llm_request_blocks blocks ON blocks.block_id=refs.block_id
             WHERE refs.manifest_id=?1 ORDER BY refs.ordinal").map_err(Self::error)?;
        let mut rows = statement.query([id]).map_err(Self::error)?;
        let mut blocks = BTreeMap::new();
        while let Some(row) = rows.next().map_err(Self::error)? {
            let ordinal: u32 = row.get(0).map_err(Self::error)?;
            let id: Option<i64> = row.get(1).map_err(Self::error)?;
            let Some(id) = id else {
                return Err(Self::error("manifest references a missing block"));
            };
            let owner: u64 = row.get(2).map_err(Self::error)?;
            let bytes: u64 = row.get(3).map_err(Self::error)?;
            let actual: u64 = row.get(4).map_err(Self::error)?;
            if bytes == 0
                || owner != trace_id.get()
                || bytes != actual
                || usize::try_from(ordinal).ok() != Some(blocks.len())
            {
                return Err(Self::error(
                    "invalid block owner, length, or reference ordinal",
                ));
            }
            blocks.insert(ordinal, BlockMeta { id, bytes });
        }
        let mut length = LengthWriter::default();
        CanonicalTraversal::new(&blocks)
            .write(&skeleton, &mut length)
            .map_err(Self::error)?;
        Ok(Some(Self {
            connection,
            trace_id,
            action_id: action_id.to_owned(),
            format_version,
            skeleton,
            blocks,
            canonical_body_bytes: length.bytes(),
        }))
    }

    pub(super) fn placeholder(value: &Value) -> Option<u32> {
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        u32::try_from(object.get("$actrail_llm_block")?.as_u64()?).ok()
    }

    pub(super) fn blob(
        &self,
        meta: &BlockMeta,
    ) -> Result<rusqlite::blob::Blob<'a>, SemanticActionStoreError> {
        let blob = self
            .connection
            .blob_open(
                DatabaseName::Main,
                "llm_request_blocks",
                "encoded_bytes",
                meta.id,
                true,
            )
            .map_err(Self::error)?;
        if blob.len() as u64 != meta.bytes {
            return Err(Self::error("block length changed during read"));
        }
        Ok(blob)
    }

    pub(super) fn error(error: impl std::fmt::Display) -> SemanticActionStoreError {
        SemanticActionStoreError::new("llm_request_content", error.to_string())
    }
}

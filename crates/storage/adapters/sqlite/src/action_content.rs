//! Resolve content from one semantic action and read one bounded HTTP body.
use std::io::Read;

use model_core::event::{ApplicationBody, DomainEvent, EventPayload};
use model_core::ids::TraceId;
use rusqlite::{OptionalExtension, params};
use semantic_action::{SemanticActionKind, SemanticEvidenceKind};
use storage_core::{
    ActionContentSource, HttpContentPage, HttpContentQuery, StorageBackend, StorageError,
};

use crate::{SqliteStorage, records};

impl SqliteStorage {
    pub(crate) fn read_action_content_source(
        &self,
        trace: TraceId,
        id: &str,
    ) -> Result<Option<ActionContentSource>, StorageError> {
        let Some(action) = StorageBackend::semantic_action_by_id(self, trace, id)? else {
            return Ok(None);
        };
        if action.kind == SemanticActionKind::LlmRequest {
            return Ok(Some(ActionContentSource::Llm {
                action_id: id.into(),
            }));
        }
        if action.kind != SemanticActionKind::HttpMessage {
            return Ok(None);
        }
        let connection = self.connection().borrow();
        let parent: Option<String> = connection
            .query_row(
                "SELECT parent.action_id FROM semantic_action_ids child
             JOIN semantic_action_links link ON link.child_action_key=child.action_key
             JOIN semantic_action_ids parent ON parent.action_key=link.parent_action_key
             JOIN semantic_actions action ON action.action_key=parent.action_key
             WHERE child.trace_id=?1 AND child.action_id=?2 AND link.trace_id=?1
               AND link.valid=1 AND link.role_code=?3 AND action.kind_code=?4
             ORDER BY parent.action_key LIMIT 1",
                params![
                    trace.get(),
                    id,
                    crate::semantic_actions::codebook::current()
                        .link_role
                        .llm_request_http_message,
                    crate::semantic_actions::codebook::current()
                        .action_kind
                        .llm_request
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(HttpBodyReader::error)?;
        if let Some(action_id) = parent {
            return Ok(Some(ActionContentSource::Llm { action_id }));
        }
        Ok(action
            .evidence
            .iter()
            .any(|e| e.kind == SemanticEvidenceKind::Event && e.role == "http.message")
            .then_some(ActionContentSource::Http))
    }

    pub(crate) fn read_http_content_page(
        &self,
        trace: TraceId,
        id: &str,
        query: &HttpContentQuery,
    ) -> Result<Option<HttpContentPage>, StorageError> {
        if query.max_bytes == 0 || query.read_budget == 0 {
            return Err(HttpBodyReader::error("invalid body budget"));
        }
        let Some(action) = StorageBackend::semantic_action_by_id(self, trace, id)? else {
            return Ok(None);
        };
        if action.kind != SemanticActionKind::HttpMessage {
            return Ok(None);
        }
        let Some(evidence) = action
            .evidence
            .iter()
            .find(|e| e.kind == SemanticEvidenceKind::Event && e.role == "http.message")
        else {
            return Ok(None);
        };
        let connection = self.connection().borrow();
        let reader = HttpBodyReader {
            connection: &connection,
            trace,
            budget: query.read_budget,
        };
        let Some((kind, text)) = reader.read(evidence.id)? else {
            return Ok(None);
        };
        if query.offset > text.len() || !text.is_char_boundary(query.offset) {
            return Err(HttpBodyReader::error("invalid HTTP body offset"));
        }
        let mut end = query.offset.saturating_add(query.max_bytes).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == query.offset && end < text.len() {
            return Err(HttpBodyReader::error(
                "page budget cannot hold next UTF-8 character",
            ));
        }
        Ok(Some(HttpContentPage {
            kind,
            text: text[query.offset..end].to_owned(),
            offset: query.offset,
            next_offset: (end < text.len()).then_some(end),
            total_bytes: text.len(),
        }))
    }
}

struct HttpBodyReader<'a> {
    connection: &'a rusqlite::Connection,
    trace: TraceId,
    budget: usize,
}

impl HttpBodyReader<'_> {
    fn error(error: impl std::fmt::Display) -> StorageError {
        StorageError::new("http_content", error.to_string())
    }

    fn read(&self, event: u64) -> Result<Option<(&'static str, String)>, StorageError> {
        let row_exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE trace_id=?1 AND event_id=?2)",
                params![self.trace.get(), event],
                |r| r.get(0),
            )
            .map_err(Self::error)?;
        if row_exists {
            let meta: Option<(i64,usize)> = self.connection.query_row("SELECT kind,length(encoded_bytes) FROM event_payload_blocks WHERE event_id=?1 AND block_order=0", [event], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(Self::error)?;
            let Some((kind, length)) = meta else {
                return Ok(None);
            };
            if length > self.budget {
                return Err(Self::error("compressed HTTP body exceeds read budget"));
            }
            let bytes: Vec<u8> = self.connection.query_row("SELECT encoded_bytes FROM event_payload_blocks WHERE event_id=?1 AND block_order=0", [event], |r| r.get(0)).map_err(Self::error)?;
            let mut decoded = Vec::new();
            zstd::stream::Decoder::new(bytes.as_slice())
                .map_err(Self::error)?
                .take(self.budget as u64 + 1)
                .read_to_end(&mut decoded)
                .map_err(Self::error)?;
            if decoded.len() > self.budget {
                return Err(Self::error("HTTP body exceeds read budget"));
            }
            let kind = match records::BlockKind::from_i64(kind) {
                Some(records::BlockKind::HttpBodyText) => "text",
                Some(records::BlockKind::HttpBodyJson) => "json",
                Some(records::BlockKind::HttpBodyBase64) => "base64",
                _ => return Err(Self::error("invalid HTTP body type")),
            };
            return Ok(Some((
                kind,
                String::from_utf8(decoded).map_err(Self::error)?,
            )));
        }
        let pending: Option<usize> = self.connection.query_row("SELECT length(encoded_frame) FROM event_record_pending WHERE trace_id=?1 AND event_id=?2",params![self.trace.get(),event],|r|r.get(0)).optional().map_err(Self::error)?;
        let mut resolve_path = |path: i64| {
            self.connection.query_row(
                "SELECT path_text FROM file_paths WHERE trace_id=?1 AND path_id=?2",
                params![self.trace.get(), path],
                |r| r.get(0),
            )
        };
        if let Some(length) = pending {
            if length > self.budget {
                return Err(Self::error("pending event exceeds read budget"));
            }
            let bytes:Vec<u8> = self.connection.query_row("SELECT encoded_frame FROM event_record_pending WHERE trace_id=?1 AND event_id=?2",params![self.trace.get(),event],|r|r.get(0)).map_err(Self::error)?;
            let value =
                records::decode_event_record_frame(self.trace.get(), &bytes, &mut resolve_path)
                    .map_err(Self::error)?;
            return Self::body(value, event);
        }
        // IDs can arrive out of order across flushes, so ranges may overlap.
        // Decode only candidate blocks, charging all of them to one request budget.
        let mut statement = self
            .connection
            .prepare(
                "SELECT block_id,codec_version,event_count,uncompressed_bytes,length(encoded_bytes)
             FROM event_record_blocks WHERE trace_id=?1 AND first_event_id<=?2 AND max_event_id>=?2
             ORDER BY first_event_id DESC",
            )
            .map_err(Self::error)?;
        let mut rows = statement
            .query(params![self.trace.get(), event])
            .map_err(Self::error)?;
        let mut remaining = self.budget;
        while let Some(row) = rows.next().map_err(Self::error)? {
            let block: i64 = row.get(0).map_err(Self::error)?;
            let version: i64 = row.get(1).map_err(Self::error)?;
            let count: usize = row.get(2).map_err(Self::error)?;
            let raw: usize = row.get(3).map_err(Self::error)?;
            let length: usize = row.get(4).map_err(Self::error)?;
            let cost = raw.max(length);
            if cost == 0 || cost > remaining {
                return Err(Self::error("event blocks exceed read budget"));
            }
            remaining -= cost;
            let bytes: Vec<u8> = self
                .connection
                .query_row(
                    "SELECT encoded_bytes FROM event_record_blocks WHERE block_id=?1",
                    [block],
                    |r| r.get(0),
                )
                .map_err(Self::error)?;
            let values = records::decode_event_record_block(
                self.trace.get(),
                version,
                count,
                raw,
                &bytes,
                &mut resolve_path,
            )
            .map_err(Self::error)?;
            if let Some(value) = values
                .into_iter()
                .find(|value| value.envelope.event_id.get() == event)
            {
                return Self::body(value, event);
            }
        }
        Ok(None)
    }

    fn body(
        value: DomainEvent,
        event: u64,
    ) -> Result<Option<(&'static str, String)>, StorageError> {
        if value.envelope.event_id.get() != event {
            return Err(Self::error("event identity mismatch"));
        }
        let EventPayload::Application(application) = value.payload else {
            return Err(Self::error("HTTP evidence is not an application event"));
        };
        Ok(application.body.map(|body| match body {
            ApplicationBody::Text(text) => ("text", text),
            ApplicationBody::Json(text) => ("json", text),
            ApplicationBody::Base64(text) => ("base64", text),
        }))
    }
}

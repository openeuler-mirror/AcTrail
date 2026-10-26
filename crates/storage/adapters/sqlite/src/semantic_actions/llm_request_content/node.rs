//! Read selected JSON nodes and bounded UTF-8 string pages from canonical blocks.

use std::{collections::BTreeMap, io::Read, rc::Rc};

use semantic_action::{
    ContentNode, ContentNodeChild, ContentNodeKind, ContentNodePage, ContentNodeQuery,
    SemanticActionStoreError,
};
use serde_json::Value;

use super::read::ContentReader;

impl ContentReader<'_> {
    pub(super) fn node(
        &self,
        query: &ContentNodeQuery,
    ) -> Result<ContentNodePage, SemanticActionStoreError> {
        if query.limit == 0 || query.preview_bytes == 0 || query.block_read_budget == 0 {
            return Err(Self::error("node limits must be positive"));
        }
        let tokens = if query.pointer.is_empty() {
            Vec::new()
        } else {
            let path = query
                .pointer
                .strip_prefix('/')
                .ok_or_else(|| Self::error("invalid JSON pointer"))?;
            path.split('/')
                .map(|token| token.replace("~1", "/").replace("~0", "~"))
                .collect()
        };
        let mut reader = NodeReader {
            content: self,
            query,
            decoded: BTreeMap::new(),
            kinds: BTreeMap::new(),
            read_bytes: 0,
        };
        let node = reader.navigate(&self.skeleton, &tokens, true)?;
        Ok(ContentNodePage {
            action_id: self.action_id.clone(),
            format_version: self.format_version,
            canonical_body_bytes: self.canonical_body_bytes,
            pointer: query.pointer.clone(),
            node,
        })
    }
}

struct NodeReader<'r, 'c> {
    content: &'r ContentReader<'c>,
    query: &'r ContentNodeQuery,
    decoded: BTreeMap<i64, Rc<Value>>,
    kinds: BTreeMap<i64, ContentNodeKind>,
    read_bytes: usize,
}

impl NodeReader<'_, '_> {
    fn resolve(&mut self, ordinal: u32) -> Result<Rc<Value>, SemanticActionStoreError> {
        let meta = self
            .content
            .blocks
            .get(&ordinal)
            .ok_or_else(|| ContentReader::error("missing block ordinal"))?;
        if let Some(value) = self.decoded.get(&meta.id) {
            return Ok(Rc::clone(value));
        }
        let bytes = usize::try_from(meta.bytes).map_err(ContentReader::error)?;
        let additional_bytes = bytes.saturating_sub(usize::from(self.kinds.contains_key(&meta.id)));
        let remaining = self.query.block_read_budget.saturating_sub(self.read_bytes);
        if additional_bytes > remaining {
            return Err(SemanticActionStoreError::new(
                "llm_request_node_budget",
                format!(
                    "node requires block of {bytes} bytes; {remaining} bytes remain in block_read_budget {}",
                    self.query.block_read_budget
                ),
            ));
        }
        let mut raw = vec![0; bytes];
        self.content
            .blob(meta)?
            .read_exact(&mut raw)
            .map_err(ContentReader::error)?;
        let value = Rc::new(serde_json::from_slice(&raw).map_err(ContentReader::error)?);
        self.read_bytes += additional_bytes;
        self.decoded.insert(meta.id, Rc::clone(&value));
        Ok(value)
    }

    fn block_kind(&mut self, ordinal: u32) -> Result<ContentNodeKind, SemanticActionStoreError> {
        let meta = self
            .content
            .blocks
            .get(&ordinal)
            .ok_or_else(|| ContentReader::error("missing block ordinal"))?;
        if let Some(value) = self.decoded.get(&meta.id) {
            return Ok(Self::kind(value));
        }
        if let Some(kind) = self.kinds.get(&meta.id) {
            return Ok(*kind);
        }
        if self.read_bytes >= self.query.block_read_budget {
            return Err(SemanticActionStoreError::new(
                "llm_request_node_budget",
                "block_read_budget exhausted while reading node descriptions",
            ));
        }
        // Canonical JSON has no leading whitespace. One byte identifies its type;
        // exact child counts become available when the user opens the block.
        let mut first = [0];
        self.content
            .blob(meta)?
            .read_exact(&mut first)
            .map_err(ContentReader::error)?;
        let kind = match first[0] {
            b'{' => ContentNodeKind::Object,
            b'[' => ContentNodeKind::Array,
            b'"' => ContentNodeKind::String,
            b't' | b'f' => ContentNodeKind::Boolean,
            b'n' => ContentNodeKind::Null,
            b'-' | b'0'..=b'9' => ContentNodeKind::Number,
            _ => return Err(ContentReader::error("invalid canonical block type")),
        };
        self.read_bytes += 1;
        self.kinds.insert(meta.id, kind);
        Ok(kind)
    }

    fn navigate(
        &mut self,
        value: &Value,
        tokens: &[String],
        skeleton: bool,
    ) -> Result<ContentNode, SemanticActionStoreError> {
        if skeleton {
            if let Some(ordinal) = ContentReader::placeholder(value) {
                let block = self.resolve(ordinal)?;
                return self.navigate(&block, tokens, false);
            }
        }
        if let Some((token, rest)) = tokens.split_first() {
            let child = match value {
                Value::Object(values) => values.get(token),
                Value::Array(values) => {
                    if token.starts_with('+') || (token.len() > 1 && token.starts_with('0')) {
                        None
                    } else {
                        token
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| values.get(index))
                    }
                }
                _ => None,
            }
            .ok_or_else(|| {
                SemanticActionStoreError::new(
                    "llm_request_node_pointer",
                    "JSON pointer does not exist",
                )
            })?;
            return self.navigate(child, rest, skeleton);
        }
        self.render(value, skeleton)
    }

    fn describe(
        &mut self,
        token: String,
        value: &Value,
        skeleton: bool,
    ) -> Result<ContentNodeChild, SemanticActionStoreError> {
        let (kind, child_count) = match skeleton
            .then(|| ContentReader::placeholder(value))
            .flatten()
        {
            Some(ordinal) => (self.block_kind(ordinal)?, None),
            None => (
                Self::kind(value),
                match value {
                    Value::Array(values) => Some(values.len()),
                    Value::Object(values) => Some(values.len()),
                    _ => None,
                },
            ),
        };
        let escaped = token.replace('~', "~0").replace('/', "~1");
        Ok(ContentNodeChild {
            pointer: format!("{}/{escaped}", self.query.pointer),
            token,
            kind,
            child_count,
        })
    }

    fn render(
        &mut self,
        value: &Value,
        skeleton: bool,
    ) -> Result<ContentNode, SemanticActionStoreError> {
        let (total, children) = match value {
            Value::Array(values) => {
                let children = values
                    .iter()
                    .enumerate()
                    .skip(self.query.offset)
                    .take(self.query.limit)
                    .map(|(index, value)| self.describe(index.to_string(), value, skeleton))
                    .collect::<Result<Vec<_>, _>>()?;
                (values.len(), children)
            }
            Value::Object(values) => {
                let mut keys: Vec<_> = values.keys().collect();
                keys.sort_unstable();
                let children = keys
                    .into_iter()
                    .skip(self.query.offset)
                    .take(self.query.limit)
                    .map(|key| self.describe(key.clone(), &values[key], skeleton))
                    .collect::<Result<Vec<_>, _>>()?;
                (values.len(), children)
            }
            Value::String(value) => {
                let start = self.query.offset;
                if start > value.len() || !value.is_char_boundary(start) {
                    return Err(ContentReader::error(
                        "string offset must be a UTF-8 boundary within the value",
                    ));
                }
                // Keep each page within its byte budget and continue at a whole character.
                let mut end = value
                    .len()
                    .min(start.saturating_add(self.query.preview_bytes));
                while !value.is_char_boundary(end) {
                    end -= 1;
                }
                if start == end && start < value.len() {
                    return Err(ContentReader::error(
                        "preview_bytes is too small for the next UTF-8 character",
                    ));
                }
                return Ok(ContentNode::Leaf {
                    value: Value::String(value[start..end].to_owned()),
                    value_bytes: value.len() as u64,
                    returned_bytes: (end - start) as u64,
                    offset: start,
                    next_offset: (end < value.len()).then_some(end),
                    truncated: start > 0 || end < value.len(),
                });
            }
            _ => {
                let bytes = value.to_string().len() as u64;
                return Ok(ContentNode::Leaf {
                    value: value.clone(),
                    value_bytes: bytes,
                    returned_bytes: bytes,
                    offset: 0,
                    next_offset: None,
                    truncated: false,
                });
            }
        };
        let next_offset = self
            .query
            .offset
            .checked_add(children.len())
            .filter(|next| *next < total);
        Ok(ContentNode::Branch {
            kind: Self::kind(value),
            total_children: total,
            offset: self.query.offset,
            limit: self.query.limit,
            next_offset,
            children,
        })
    }

    fn kind(value: &Value) -> ContentNodeKind {
        match value {
            Value::Null => ContentNodeKind::Null,
            Value::Bool(_) => ContentNodeKind::Boolean,
            Value::Number(_) => ContentNodeKind::Number,
            Value::String(_) => ContentNodeKind::String,
            Value::Array(_) => ContentNodeKind::Array,
            Value::Object(_) => ContentNodeKind::Object,
        }
    }
}

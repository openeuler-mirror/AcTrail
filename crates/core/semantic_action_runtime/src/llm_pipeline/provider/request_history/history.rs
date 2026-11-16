use std::ops::Range;
use std::sync::Arc;

use serde_json::Value;

use super::super::RequestContentBlock;
use super::model::{
    HistoryItem, RequestHistory as ContextHistory, RequestHistoryLimit,
    RequestHistoryObservation as HistoryObservation,
};
use crate::llm_pipeline::utils::CanonicalJsonWriter;

pub(super) struct JsonHistoryReader {
    pub(super) max_items: usize,
    pub(super) max_bytes: usize,
}

impl JsonHistoryReader {
    pub(super) fn snapshot(&self, body: &Value) -> HistoryObservation {
        let Some(items) = body
            .get("messages")
            .or_else(|| body.get("input"))
            .and_then(Value::as_array)
        else {
            return HistoryObservation::Absent;
        };
        if items.len() > self.max_items {
            return HistoryObservation::LimitExceeded(RequestHistoryLimit::Entries(items.len()));
        }
        let mut remaining = self.max_bytes;
        let system = if let Some(system) = body.get("system") {
            let Ok(encoded) =
                CanonicalJsonWriter::new(remaining as u64).serialize_history_content(system)
            else {
                return HistoryObservation::LimitExceeded(RequestHistoryLimit::BytesUnknown);
            };
            remaining = remaining.saturating_sub(encoded.len());
            Some(Arc::from(encoded))
        } else {
            None
        };
        let mut history = Vec::with_capacity(items.len());
        let mut remaining_tool_calls = self.max_items;
        for item in items {
            let writer = CanonicalJsonWriter::new(remaining as u64);
            let canonical = if item.get("tool_calls").is_some() {
                writer.serialize_history_message(item)
            } else {
                writer.serialize_history_content(item.get("content").unwrap_or(item))
            };
            let Ok(canonical) = canonical else {
                return HistoryObservation::LimitExceeded(RequestHistoryLimit::BytesUnknown);
            };
            remaining = remaining.saturating_sub(canonical.len());
            let mut text = String::new();
            let parts = match self.append_text(item.get("content"), &mut text, &mut remaining) {
                Ok(parts) => parts,
                Err(limit) => return HistoryObservation::LimitExceeded(limit),
            };
            let tool_calls = match self.append_tool_calls(
                item,
                &mut text,
                &mut remaining,
                &mut remaining_tool_calls,
            ) {
                Ok(calls) => calls,
                Err(observation) => return observation,
            };
            history.push(HistoryItem {
                role: item
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                canonical,
                text,
                parts,
                tool_calls,
            });
        }
        HistoryObservation::Present(ContextHistory {
            items: Arc::from(history),
            system,
        })
    }

    fn append_text(
        &self,
        content: Option<&Value>,
        text: &mut String,
        remaining: &mut usize,
    ) -> Result<Vec<Range<usize>>, RequestHistoryLimit> {
        let Some(content) = content else {
            return Ok(Vec::new());
        };
        match content {
            Value::String(value) => {
                let start = text.len();
                self.append_segment(value, text, remaining)?;
                Ok(if value.is_empty() {
                    Vec::new()
                } else {
                    vec![start..text.len()]
                })
            }
            Value::Array(blocks) => {
                if blocks.len() > self.max_items {
                    return Err(RequestHistoryLimit::Entries(blocks.len()));
                }
                let mut parts = Vec::new();
                for block in blocks {
                    if let Some(value) = RequestContentBlock::new(block).text() {
                        let start = text.len();
                        self.append_segment(value, text, remaining)?;
                        if !value.is_empty() {
                            parts.push(start..text.len());
                        }
                    }
                }
                Ok(parts)
            }
            _ => Ok(Vec::new()),
        }
    }

    fn append_segment(
        &self,
        value: &str,
        text: &mut String,
        remaining: &mut usize,
    ) -> Result<(), RequestHistoryLimit> {
        if value.len() > *remaining {
            return Err(RequestHistoryLimit::Bytes(value.len()));
        }
        *remaining -= value.len();
        text.push_str(value);
        Ok(())
    }

    fn append_tool_calls(
        &self,
        item: &Value,
        text: &mut String,
        remaining: &mut usize,
        remaining_calls: &mut usize,
    ) -> Result<Vec<Range<usize>>, HistoryObservation> {
        let Some(calls) = item.get("tool_calls") else {
            return Ok(Vec::new());
        };
        let Some(calls) = calls.as_array() else {
            return Err(HistoryObservation::Absent);
        };
        if calls.len() > *remaining_calls {
            return Err(HistoryObservation::LimitExceeded(
                RequestHistoryLimit::Entries(calls.len()),
            ));
        }
        *remaining_calls -= calls.len();
        let mut ranges = Vec::with_capacity(calls.len());
        for call in calls {
            if item.get("role").and_then(Value::as_str) != Some("assistant")
                || call.get("type").and_then(Value::as_str) != Some("function")
            {
                return Err(HistoryObservation::Absent);
            }
            let Some(function) = call.get("function") else {
                return Err(HistoryObservation::Absent);
            };
            let Some(name) = function.get("name").and_then(Value::as_str) else {
                return Err(HistoryObservation::Absent);
            };
            let Some(arguments) = function.get("arguments").and_then(Value::as_str) else {
                return Err(HistoryObservation::Absent);
            };
            if name.is_empty() || arguments.is_empty() {
                return Err(HistoryObservation::Absent);
            }
            let Some(bytes) = name
                .len()
                .checked_add(arguments.len())
                .and_then(|bytes| bytes.checked_add(2))
            else {
                return Err(HistoryObservation::LimitExceeded(
                    RequestHistoryLimit::BytesUnknown,
                ));
            };
            if bytes > *remaining {
                return Err(HistoryObservation::LimitExceeded(
                    RequestHistoryLimit::Bytes(bytes),
                ));
            }
            *remaining -= bytes;
            let start = text.len();
            // A transcript can omit wire call IDs while preserving the full
            // invocation. Keep IDs in canonical history, not in this witness.
            text.push_str(name);
            text.push('(');
            text.push_str(arguments);
            text.push(')');
            ranges.push(start..text.len());
        }
        Ok(ranges)
    }
}

use serde_json::Value;

use crate::llm_pipeline::provider::{RequestContentBlock, RequestContextShape};

pub(super) enum CarrierContent<'a> {
    Absent,
    Text(&'a str),
    Blocks(&'a [Value]),
}

/// Borrows either JSON history format with its wire-specific item unit.
pub(super) struct JsonCarrierAdapter<'a> {
    items: &'a [Value],
    item_unit: &'static str,
}

impl<'a> JsonCarrierAdapter<'a> {
    pub(super) fn from_body(body: &'a Value) -> Option<Self> {
        let (items, item_unit) = body
            .get("messages")
            .and_then(Value::as_array)
            .map(|items| (items, "message"))
            .or_else(|| {
                body.get("input")
                    .and_then(Value::as_array)
                    .map(|items| (items, "input_item"))
            })?;
        Some(Self { items, item_unit })
    }

    pub(super) fn items(&self) -> &'a [Value] {
        self.items
    }

    pub(super) fn shape(&self) -> RequestContextShape {
        RequestContextShape {
            item_count: self.items.len(),
            item_unit: self.item_unit,
        }
    }

    pub(super) fn role(item: &Value) -> Option<&str> {
        item.get("role").and_then(Value::as_str)
    }

    pub(super) fn content(item: &Value) -> CarrierContent<'_> {
        match item.get("content") {
            Some(Value::String(text)) => CarrierContent::Text(text),
            Some(Value::Array(blocks)) => CarrierContent::Blocks(blocks),
            _ => CarrierContent::Absent,
        }
    }

    pub(super) fn block_text(block: &Value) -> Option<&str> {
        RequestContentBlock::new(block).text()
    }

    pub(super) fn is_tool_result(block: &Value) -> bool {
        RequestContentBlock::new(block).is_tool_result()
    }
}

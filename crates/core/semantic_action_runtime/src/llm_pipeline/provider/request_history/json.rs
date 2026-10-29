use super::super::OpaqueContextArtifact;
use serde_json::Value;

use super::adapter::{CarrierContent, JsonCarrierAdapter};
use super::carrier::CarrierReader;
use super::history::JsonHistoryReader;
use super::model::{CarrierSnapshotObservation, RequestCarrierSnapshot, RequestHistoryObservation};

/// A request's immutable JSON evidence, decoded only when a matcher needs it.
pub(in crate::llm_pipeline) struct RequestHistorySource {
    body: Value,
}

/// One matching pass borrows its positions and JSON items from immutable evidence.
pub(in crate::llm_pipeline) struct RequestCarrierView<'a> {
    snapshot: &'a RequestCarrierSnapshot,
    adapter: Option<JsonCarrierAdapter<'a>>,
}

impl RequestHistorySource {
    pub(in crate::llm_pipeline::provider) fn new(body: Value) -> Self {
        Self { body }
    }
    pub(in crate::llm_pipeline) fn snapshot(
        &self,
        max_items: usize,
        max_bytes: usize,
    ) -> RequestHistoryObservation {
        JsonHistoryReader {
            max_items,
            max_bytes,
        }
        .snapshot(&self.body)
    }

    pub(in crate::llm_pipeline) fn carriers(
        &self,
        max_items: usize,
        max_bytes: usize,
    ) -> CarrierSnapshotObservation {
        let reader = CarrierReader {
            max_context_items: max_items,
            max_context_scan_bytes: max_bytes,
        };
        if let Some(adapter) = JsonCarrierAdapter::from_body(&self.body) {
            reader.carrier_snapshot(&adapter)
        } else {
            CarrierSnapshotObservation::Absent
        }
    }
    pub(in crate::llm_pipeline) fn carrier_view<'a>(
        &'a self,
        snapshot: &'a RequestCarrierSnapshot,
    ) -> RequestCarrierView<'a> {
        RequestCarrierView {
            snapshot,
            adapter: JsonCarrierAdapter::from_body(&self.body),
        }
    }

    pub(in crate::llm_pipeline) fn opaque_artifact(
        &self,
        max_items: usize,
    ) -> Option<OpaqueContextArtifact> {
        let items = self.body.get("input")?.as_array()?;
        if items.len() > max_items {
            return None;
        }
        let mut artifact_count = 0;
        let mut artifact = None;
        let mut last_user = None;
        for (index, item) in items.iter().enumerate() {
            if matches!(
                item.get("role").and_then(Value::as_str),
                Some("user" | "human")
            ) {
                last_user = Some(index);
            }
            if item.get("type").and_then(Value::as_str) != Some("compaction") {
                continue;
            }
            artifact_count += 1;
            artifact = item
                .get("encrypted_content")
                .and_then(Value::as_str)
                .filter(|content| !content.is_empty())
                .map(|content| (index, content.len()));
        }
        let (index, artifact_bytes) = artifact?;
        if artifact_count != 1 {
            return None;
        }
        Some(OpaqueContextArtifact {
            item_count: items.len(),
            item_unit: "input_item",
            artifact_path: format!("$.input[{index}].encrypted_content"),
            artifact_bytes,
            has_following_user: last_user.is_some_and(|user| user > index),
        })
    }
}

impl<'a> RequestCarrierView<'a> {
    pub(in crate::llm_pipeline) fn carrier(&self, ordinal: usize) -> Option<(&'a str, usize)> {
        let position = self.snapshot.carriers.get(ordinal)?;
        let item = self.adapter.as_ref()?.items().get(position.item_index)?;
        let text = match (JsonCarrierAdapter::content(item), position.block_index) {
            (CarrierContent::Text(text), None) => text,
            (CarrierContent::Blocks(blocks), Some(index)) => {
                JsonCarrierAdapter::block_text(blocks.get(index)?)?
            }
            _ => return None,
        };
        Some((text, position.item_index))
    }
}

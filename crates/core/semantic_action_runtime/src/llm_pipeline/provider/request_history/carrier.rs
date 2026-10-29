use serde_json::Value;

use super::adapter::{CarrierContent, JsonCarrierAdapter};
use super::model::{
    CarrierPosition, CarrierSnapshotObservation, RequestCarrierSnapshot, RequestHistoryLimit,
};

pub(super) struct CarrierReader {
    pub(super) max_context_items: usize,
    pub(super) max_context_scan_bytes: usize,
}

impl CarrierReader {
    pub(super) fn carrier_snapshot(
        &self,
        adapter: &JsonCarrierAdapter<'_>,
    ) -> CarrierSnapshotObservation {
        let items = adapter.items();
        if items.len() > self.max_context_items {
            return CarrierSnapshotObservation::LimitExceeded(RequestHistoryLimit::Entries(
                items.len(),
            ));
        }
        let Some(last_user_index) = items.iter().rposition(Self::is_real_user_item) else {
            return CarrierSnapshotObservation::Absent;
        };
        let mut carriers = Vec::new();
        let mut scanned_bytes = 0_usize;
        for (item_index, item) in items.iter().enumerate() {
            // Tool-loop summaries can follow the active user message.
            let eligible = match JsonCarrierAdapter::role(item) {
                Some("assistant") => true,
                Some("user" | "human") => item_index < last_user_index,
                _ => false,
            };
            if !eligible {
                continue;
            }
            if let Err(limit) =
                self.append_item_carriers(item, item_index, &mut carriers, &mut scanned_bytes)
            {
                return CarrierSnapshotObservation::LimitExceeded(limit);
            }
        }
        if carriers.is_empty() {
            return CarrierSnapshotObservation::Absent;
        }
        CarrierSnapshotObservation::Present(RequestCarrierSnapshot {
            shape: adapter.shape(),
            carriers,
        })
    }

    fn append_item_carriers(
        &self,
        item: &Value,
        item_index: usize,
        carriers: &mut Vec<CarrierPosition>,
        scanned_bytes: &mut usize,
    ) -> Result<(), RequestHistoryLimit> {
        match JsonCarrierAdapter::content(item) {
            CarrierContent::Text(text) => self.append_carrier(
                text,
                CarrierPosition {
                    item_index,
                    block_index: None,
                },
                carriers,
                scanned_bytes,
            ),
            CarrierContent::Blocks(blocks) => {
                if blocks.len() > self.max_context_items {
                    return Err(RequestHistoryLimit::Entries(blocks.len()));
                }
                for (block_index, block) in blocks.iter().enumerate() {
                    if let Some(text) = JsonCarrierAdapter::block_text(block) {
                        self.append_carrier(
                            text,
                            CarrierPosition {
                                item_index,
                                block_index: Some(block_index),
                            },
                            carriers,
                            scanned_bytes,
                        )?;
                    }
                    if carriers.len() > self.max_context_items {
                        return Err(RequestHistoryLimit::Entries(carriers.len()));
                    }
                }
                Ok(())
            }
            CarrierContent::Absent => Ok(()),
        }
    }

    fn append_carrier(
        &self,
        text: &str,
        position: CarrierPosition,
        carriers: &mut Vec<CarrierPosition>,
        scanned_bytes: &mut usize,
    ) -> Result<(), RequestHistoryLimit> {
        if text.is_empty() {
            return Ok(());
        }
        *scanned_bytes = scanned_bytes.saturating_add(text.len());
        if *scanned_bytes > self.max_context_scan_bytes {
            return Err(RequestHistoryLimit::Bytes(*scanned_bytes));
        }
        if carriers.len() >= self.max_context_items {
            return Err(RequestHistoryLimit::Entries(
                carriers.len().saturating_add(1),
            ));
        }
        carriers.push(position);
        Ok(())
    }

    fn is_real_user_item(item: &Value) -> bool {
        if !matches!(JsonCarrierAdapter::role(item), Some("user" | "human")) {
            return false;
        }
        !matches!(JsonCarrierAdapter::content(item), CarrierContent::Blocks(blocks)
            if !blocks.is_empty() && blocks.iter().all(JsonCarrierAdapter::is_tool_result))
    }
}

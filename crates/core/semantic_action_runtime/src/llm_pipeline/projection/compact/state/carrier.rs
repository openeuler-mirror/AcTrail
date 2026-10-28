use std::sync::OnceLock;

use crate::llm_pipeline::provider::RequestCarrierSnapshot;

/// Positions borrow the pending adoption's immutable source. Decoded string
/// views are prepared lazily and released with this pending adoption entry.
pub(in crate::llm_pipeline::projection::compact) struct PreparedCarrierSnapshot {
    pub(in crate::llm_pipeline::projection::compact) snapshot: RequestCarrierSnapshot,
    pub(in crate::llm_pipeline::projection::compact) decoded_text: Vec<OnceLock<Option<String>>>,
}

impl PreparedCarrierSnapshot {
    pub(in crate::llm_pipeline::projection::compact) fn new(
        snapshot: RequestCarrierSnapshot,
    ) -> Self {
        let decoded_text = (0..snapshot.len()).map(|_| OnceLock::new()).collect();
        Self {
            snapshot,
            decoded_text,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn len(&self) -> usize {
        self.snapshot.len()
    }
}

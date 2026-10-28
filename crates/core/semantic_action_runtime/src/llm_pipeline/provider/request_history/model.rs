use std::ops::Range;
use std::sync::Arc;

use super::super::RequestContextShape;

/// Provider-independent budgets, enforced before materializing request evidence.
#[derive(Clone, Copy)]
pub(in crate::llm_pipeline) enum RequestHistoryLimit {
    Entries(usize),
    Bytes(usize),
    BytesUnknown,
}

#[derive(Clone)]
pub(in crate::llm_pipeline) struct RequestHistory {
    pub(in crate::llm_pipeline) items: Arc<[HistoryItem]>,
    pub(in crate::llm_pipeline) system: Option<Arc<str>>,
}

pub(in crate::llm_pipeline) struct HistoryItem {
    pub(in crate::llm_pipeline) role: String,
    pub(in crate::llm_pipeline) canonical: String,
    pub(in crate::llm_pipeline) text: String,
    pub(in crate::llm_pipeline) parts: Vec<Range<usize>>,
    pub(in crate::llm_pipeline) tool_calls: Vec<Range<usize>>,
}

#[derive(Clone)]
pub(in crate::llm_pipeline) enum RequestHistoryObservation {
    Absent,
    Present(RequestHistory),
    LimitExceeded(RequestHistoryLimit),
}

pub(in crate::llm_pipeline) struct RequestCarrierSnapshot {
    pub(in crate::llm_pipeline) shape: RequestContextShape,
    pub(super) carriers: Vec<CarrierPosition>,
}

pub(super) struct CarrierPosition {
    pub(super) item_index: usize,
    pub(super) block_index: Option<usize>,
}

impl RequestCarrierSnapshot {
    pub(in crate::llm_pipeline) fn len(&self) -> usize {
        self.carriers.len()
    }
}

pub(in crate::llm_pipeline) enum CarrierSnapshotObservation {
    Absent,
    Present(RequestCarrierSnapshot),
    LimitExceeded(RequestHistoryLimit),
}

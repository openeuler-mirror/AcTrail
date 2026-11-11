//! Bounded recent-action selection, independent of the lightweight timeline view.
use model_core::{ids::TraceId, process::ProcessIdentity};
use semantic_action::SemanticActionKind;
use std::time::SystemTime;

pub struct RecentActionsQuery {
    pub trace_id: TraceId,
    pub process: ProcessIdentity,
    pub kinds: Vec<SemanticActionKind>,
    pub start: SystemTime,
    pub end: SystemTime,
    /// Maximum IDs returned, in descending start-time and storage-key order.
    pub limit: usize,
}

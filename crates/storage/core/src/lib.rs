//! Unified storage backend facade.

mod backend;
mod error;
mod transaction;

pub use alert_contract::{
    AlertDefinition, AlertDefinitionId, AlertDraft, AlertId, AlertListLimit, AlertRecord,
    AlertSeverity, AlertStoreError, AlertStoreErrorKind, AlertView,
};
pub use backend::{
    SemanticActionChildPage, SemanticActionChildPageQuery, SemanticActionChildRow,
    SemanticActionDisplayRootChildPage, SemanticActionDisplayRootChildRow, SemanticActionSummary,
    StorageBackend, StorageOpenMode,
};
pub use error::StorageError;
pub use store_read_contract::filters::TraceFilter;
pub use store_read_contract::payloads::{PayloadRowLimit, PayloadSegmentQuery};
pub use store_retention_contract::cleanup::RetentionCandidate;
pub use store_retention_contract::tombstone::TraceTombstone;
pub use store_snapshot_contract::lease::{TraceLease, TraceLeasePurpose, TraceLeaseToken};
pub use store_snapshot_contract::view::SnapshotView;
pub use transaction::StorageTransaction;

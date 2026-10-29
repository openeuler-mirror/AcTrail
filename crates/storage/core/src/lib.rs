//! Unified storage backend facade.

mod action_content;
mod backend;
pub use action_content::{ActionContentSource, HttpContentPage, HttpContentQuery};
mod error;
mod idle;
mod payload_retention;
mod timeline;
mod trace_page;
mod transaction;
pub use idle::{IdleInterval, IdlePage, IdleQuery};
pub use semantic_action::{
    ContentNode, ContentNodeChild, ContentNodeKind, ContentNodePage, ContentNodeQuery,
};
pub use timeline::{TimelineBounds, TimelineCursor, TimelinePage, TimelineQuery};
pub use trace_page::{TraceCounts, TracePage, TracePageQuery};

pub use alert_contract::{
    AlertDefinition, AlertDefinitionId, AlertDraft, AlertId, AlertListLimit, AlertRecord,
    AlertSeverity, AlertStoreError, AlertStoreErrorKind, AlertView,
};
pub use backend::{
    SemanticActionChildPage, SemanticActionChildPageQuery, SemanticActionChildRow,
    SemanticActionDisplayPathEntry, SemanticActionDisplayRootChildPage,
    SemanticActionDisplayRootChildRow, SemanticActionSummary, SemanticActionTraceRevision,
    StorageBackend, StorageOpenMode, TlsFlowDiagnostic,
};
pub use error::StorageError;
pub use payload_retention::PayloadRetentionLimits;
pub use store_read_contract::filters::TraceFilter;
pub use store_read_contract::payloads::{PayloadRowLimit, PayloadSegmentQuery};
pub use store_retention_contract::cleanup::RetentionCandidate;
pub use store_retention_contract::tombstone::TraceTombstone;
pub use store_snapshot_contract::lease::{TraceLease, TraceLeasePurpose, TraceLeaseToken};
pub use store_snapshot_contract::view::SnapshotView;
pub use transaction::StorageTransaction;

mod cgroup_v1_bindings;
mod cgroup_v2_bindings;
pub use cgroup_v1_bindings::CgroupV1BindingStore;
pub use cgroup_v2_bindings::CgroupV2BindingStore;

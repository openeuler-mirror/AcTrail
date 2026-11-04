//! Defines configuration, trace context, and snapshot identifiers for the shared ABI.

use abi_stable::{
    StableAbi,
    std_types::{RString, RVec},
};

/// Complete integration-owned configuration, forwarded without field parsing.
/// Each integration defines and validates its own JSON schema at creation.
#[repr(C)]
#[derive(Debug, StableAbi)]
pub struct BackendConfig {
    pub json: RString,
}

/// Context bound to a backend instance for one checkpoint connection.
/// The integration selects its target agent using its own configuration.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct SnapshotContext {
    /// AcTrail trace identity; the integration binds the agent's session separately.
    pub trace_id: RString,
}

/// Snapshot identifier defined and interpreted by the issuing backend instance.
/// Callers retain and pass the identifier without parsing or transforming it.
#[repr(C)]
#[derive(Debug, StableAbi)]
pub struct SnapshotId(pub RVec<u8>);

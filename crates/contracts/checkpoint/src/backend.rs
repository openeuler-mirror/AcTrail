//! Declares backend operations and the dynamic library creation entry.

// abi_stable 0.11 generates a nested InterfaceType impl in sabi_trait expansion.
#![allow(non_local_definitions)]

use abi_stable::{
    StableAbi,
    library::RootModule,
    sabi_trait,
    sabi_types::VersionStrings,
    std_types::{RBox, RResult},
};

use crate::{BackendConfig, BackendError, SnapshotContext, SnapshotId};

/// Restores one configured resource scope, such as context, files, or an API.
///
/// A composite backend owns child backends implementing this same trait and
/// associates its own snapshot IDs with their snapshots. Child scheduling
/// and snapshot associations belong to the composite implementation.
/// It passes the connection context to each child backend.
///
/// Configuration belongs to the concrete backend's construction interface.
/// Calls are synchronous and exclusive on each instance; RPC calls may block
/// until a reply arrives. Scheduling belongs to the caller or composite backend.
/// Operational failures are returned as errors and must not unwind across the ABI.
#[sabi_trait]
pub trait RestorableCheckpointBackend: Send {
    /// Establishes checkpoint integration and binds this instance to the context.
    /// Subsequent operations use that binding until disconnect. Concurrent traces
    /// use separate backend instances. The integration binds its target session
    /// separately from the trace identity. Snapshot operations require connection.
    fn connect(&mut self, context: &SnapshotContext) -> RResult<(), BackendError>;

    /// Requests a snapshot using the backend's capture policy and returns its ID.
    /// The policy may select the latest complete context boundary.
    /// On failure, the backend remains responsible for any resources for which
    /// no snapshot ID was returned.
    fn snapshot(&mut self) -> RResult<SnapshotId, BackendError>;

    /// Requests restoration from a snapshot, retaining its ID for later use.
    /// Failure may leave resources partly changed.
    fn restore(&mut self, snapshot_id: &SnapshotId) -> RResult<(), BackendError>;

    /// Requests deletion of one snapshot. Once accepted, its ID must no longer
    /// be used for restoration. The backend connection remains active.
    fn delete(&mut self, snapshot_id: &SnapshotId) -> RResult<(), BackendError>;

    /// Ends checkpoint integration and releases connection or session resources.
    /// Pairs with connect; individual snapshot removal is handled by delete.
    #[sabi(last_prefix_field)]
    fn disconnect(&mut self) -> RResult<(), BackendError>;
}

/// Owned ABI-safe object shared by in-process and dynamically loaded backends.
pub type CheckpointBackend = RestorableCheckpointBackend_TO<'static, RBox<()>>;

/// Dynamic library entry table. Each integration exports this shared ABI.
///
/// Loaders for multiple integration libraries must initialize this table through
/// each library's header. RootModule's load_from methods cache one library per
/// module type and therefore cannot select multiple integrations of this type.
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = CheckpointModuleRef)))]
pub struct CheckpointModule {
    /// Creates a configured backend. The caller must then call connect before use.
    /// The integration owns configuration parsing and validation.
    #[sabi(last_prefix_field)]
    pub create: extern "C" fn(BackendConfig) -> RResult<CheckpointBackend, BackendError>,
}

// Shared ABI metadata for library exports and load-time layout validation.
impl RootModule for CheckpointModuleRef {
    abi_stable::declare_root_module_statics! {CheckpointModuleRef}

    const BASE_NAME: &'static str = "actrail_ckpt";
    const NAME: &'static str = "actrail_checkpoint";
    const VERSION_STRINGS: VersionStrings = abi_stable::package_version_strings!();
}

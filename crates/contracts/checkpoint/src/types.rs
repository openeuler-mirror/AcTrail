//! Defines configuration, trace context, and snapshot identifiers for the shared ABI.

use abi_stable::{
    StableAbi,
    std_types::{RString, RVec},
};

/// What a backend snapshots, which decides how it joins a Trace.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
pub enum BackendKind {
    /// Protects the directories of launch scopes, and joins a Trace when a scope names it.
    Filesystem,
    /// Has no scopes and joins every Trace.
    Context,
}

/// Complete integration-owned configuration, forwarded without field parsing.
/// Each integration defines and validates its own JSON schema at creation.
#[repr(C)]
#[derive(Debug, StableAbi)]
pub struct BackendConfig {
    pub json: RString,
}

/// Context bound to a backend instance for one checkpoint connection: whose work
/// it snapshots and which part of it. A composite backend passes each child backend
/// the scopes that child owns.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct SnapshotContext {
    /// AcTrail trace identity.
    pub trace_id: RString,
    pub trace_process: TraceProcess,
    pub scopes: RVec<SnapshotScope>,
}

/// The Trace root, stopped at the launch barrier while the backend connects.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct TraceProcess {
    pub host_pid: u32,
    /// From `/proc/<pid>/stat`; together with the PID it identifies the process.
    pub start_time_ticks: u64,
    pub pid_namespace: RString,
    pub mount_namespace: RString,
    /// Working directory, in the Agent's view of the filesystem.
    pub cwd: RString,
}

/// A protected directory after nesting is resolved; the roots of nested scopes are
/// already in `excludes`.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct SnapshotScope {
    pub id: RString,
    /// Absolute path in the Agent's view of the filesystem.
    pub root: RString,
    pub excludes: RVec<RString>,
}

/// Snapshot identifier defined and interpreted by the issuing backend instance.
/// Callers retain and pass the identifier without parsing or transforming it.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
pub struct SnapshotId(pub RVec<u8>);

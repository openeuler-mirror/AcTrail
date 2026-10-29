//! Durable read-only binding to a runtime-owned host cgroup.

use std::path::PathBuf;
use std::time::SystemTime;

use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::ids::TraceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CgroupV2BindingState {
    Active,
    Stale,
    Closed,
}

impl CgroupV2BindingState {
    pub const fn as_storage_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Closed => "closed",
        }
    }

    pub fn from_storage_str(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(Self::Active),
            "stale" => Some(Self::Stale),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }

    pub const fn is_live(self) -> bool {
        matches!(self, Self::Active | Self::Stale)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CgroupV2BindingStaleReason {
    HostBootChanged,
    BoundaryMissing,
    BoundaryIdentityChanged,
    ContainerIdentityMismatch,
    RootMoved,
    RequiredCounterUnavailable,
    PermissionDenied,
    RepeatedReadFailure,
}

impl CgroupV2BindingStaleReason {
    pub const fn as_storage_str(self) -> &'static str {
        match self {
            Self::HostBootChanged => "host_boot_changed",
            Self::BoundaryMissing => "boundary_missing",
            Self::BoundaryIdentityChanged => "boundary_identity_changed",
            Self::ContainerIdentityMismatch => "container_identity_mismatch",
            Self::RootMoved => "root_moved",
            Self::RequiredCounterUnavailable => "required_counter_unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::RepeatedReadFailure => "repeated_read_failure",
        }
    }

    pub fn from_storage_str(raw: &str) -> Option<Self> {
        match raw {
            "host_boot_changed" => Some(Self::HostBootChanged),
            "boundary_missing" => Some(Self::BoundaryMissing),
            "boundary_identity_changed" => Some(Self::BoundaryIdentityChanged),
            "container_identity_mismatch" => Some(Self::ContainerIdentityMismatch),
            "root_moved" => Some(Self::RootMoved),
            "required_counter_unavailable" => Some(Self::RequiredCounterUnavailable),
            "permission_denied" => Some(Self::PermissionDenied),
            "repeated_read_failure" => Some(Self::RepeatedReadFailure),
            _ => None,
        }
    }
}

pub use model_core::host_identity::HostBootId;

/// A read-only cgroup v2 binding to a container runtime's cgroup.
///
/// A binding means AcTrail did not create the cgroup (the table name and the
/// `cgroup.v2.external` config key keep the older "external" wording). Only
/// container targets whose runtime and container ID are recognized are bound,
/// and only when `cgroup.v2.external.policy` allows it. Stored in
/// `trace_external_cgroup_v2_bindings`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupV2Binding {
    pub trace_id: TraceId,
    pub runtime: ContainerRuntime,
    pub container_id: NormalizedContainerId,
    pub relative_path: PathBuf,
    pub cgroup_device: u64,
    pub cgroup_inode: u64,
    pub host_boot_id: HostBootId,
    pub lifecycle_state: CgroupV2BindingState,
    pub stale_reason: Option<CgroupV2BindingStaleReason>,
    pub consecutive_failures: u32,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub last_good_at: Option<SystemTime>,
    pub closed_at: Option<SystemTime>,
}

impl CgroupV2Binding {
    #[allow(clippy::too_many_arguments)]
    pub fn active(
        trace_id: TraceId,
        runtime: ContainerRuntime,
        container_id: NormalizedContainerId,
        relative_path: impl Into<PathBuf>,
        cgroup_device: u64,
        cgroup_inode: u64,
        host_boot_id: HostBootId,
        created_at: SystemTime,
    ) -> Self {
        Self {
            trace_id,
            runtime,
            container_id,
            relative_path: relative_path.into(),
            cgroup_device,
            cgroup_inode,
            host_boot_id,
            lifecycle_state: CgroupV2BindingState::Active,
            stale_reason: None,
            consecutive_failures: 0,
            created_at,
            updated_at: created_at,
            last_good_at: None,
            closed_at: None,
        }
    }
}

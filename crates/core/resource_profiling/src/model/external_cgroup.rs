//! Durable read-only binding to a runtime-owned host cgroup.

use std::path::PathBuf;
use std::time::SystemTime;

use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::ids::TraceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalBindingState {
    Active,
    Stale,
    Closed,
}

impl ExternalBindingState {
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
pub enum ExternalBindingStaleReason {
    HostBootChanged,
    BoundaryMissing,
    BoundaryIdentityChanged,
    ContainerIdentityMismatch,
    RootMoved,
    RequiredCounterUnavailable,
    PermissionDenied,
    RepeatedReadFailure,
}

impl ExternalBindingStaleReason {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalCgroupBinding {
    pub trace_id: TraceId,
    pub runtime: ContainerRuntime,
    pub container_id: NormalizedContainerId,
    pub relative_path: PathBuf,
    pub cgroup_device: u64,
    pub cgroup_inode: u64,
    pub host_boot_id: HostBootId,
    pub lifecycle_state: ExternalBindingState,
    pub stale_reason: Option<ExternalBindingStaleReason>,
    pub consecutive_failures: u32,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub last_good_at: Option<SystemTime>,
    pub closed_at: Option<SystemTime>,
}

impl ExternalCgroupBinding {
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
            lifecycle_state: ExternalBindingState::Active,
            stale_reason: None,
            consecutive_failures: 0,
            created_at,
            updated_at: created_at,
            last_good_at: None,
            closed_at: None,
        }
    }
}

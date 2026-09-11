//! Durable read-only binding to a runtime-owned host cgroup.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::container::{ContainerRuntime, NormalizedContainerId};
use crate::ids::{EventId, TraceId};

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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HostBootId([u8; 16]);

impl HostBootId {
    pub const BYTE_COUNT: usize = 16;

    pub const fn from_bytes(bytes: [u8; Self::BYTE_COUNT]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_COUNT] {
        &self.0
    }
}

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
    pub final_event_id: Option<EventId>,
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
            final_event_id: None,
        }
    }
}

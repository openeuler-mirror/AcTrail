//! Shared helpers for runtime-owned host cgroup identity verification.

use std::fs;
use std::io;
use std::path::Path;

use super::CgroupV2BindingError;
use crate::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use crate::model::cgroup_v2_binding::HostBootId;

pub(crate) struct HostBindingEnvironment;
impl HostBindingEnvironment {
    pub(crate) fn read_host_boot_id(procfs_root: &Path) -> io::Result<HostBootId> {
        let raw = fs::read_to_string(procfs_root.join("sys/kernel/random/boot_id"))?;
        raw.trim()
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error}")))
    }

    pub(crate) fn stale_reason_for_open_error(
        error: CgroupV2BindingError,
    ) -> CgroupV2BindingStaleReason {
        match error {
            CgroupV2BindingError::ContainerIdentityMissing
            | CgroupV2BindingError::ContainerIdentityMismatch
            | CgroupV2BindingError::Malformed(_) => {
                CgroupV2BindingStaleReason::ContainerIdentityMismatch
            }
            CgroupV2BindingError::RootMoved => CgroupV2BindingStaleReason::RootMoved,
            CgroupV2BindingError::Io { source, .. }
                if source.kind() == io::ErrorKind::PermissionDenied =>
            {
                CgroupV2BindingStaleReason::PermissionDenied
            }
            CgroupV2BindingError::Io { .. } => CgroupV2BindingStaleReason::BoundaryMissing,
            CgroupV2BindingError::Unsupported(_) => {
                CgroupV2BindingStaleReason::RequiredCounterUnavailable
            }
            CgroupV2BindingError::ExpectedStartTimeMissing
            | CgroupV2BindingError::PidReuse
            | CgroupV2BindingError::MembershipUnstable => {
                CgroupV2BindingStaleReason::ContainerIdentityMismatch
            }
        }
    }
}

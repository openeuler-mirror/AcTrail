//! Shared helpers for runtime-owned host cgroup identity verification.

use std::fs;
use std::io;
use std::path::Path;

use super::HostCgroupBindingError;
use crate::model::external_cgroup::ExternalBindingStaleReason;
use crate::model::external_cgroup::HostBootId;

pub(crate) struct HostBindingEnvironment;
impl HostBindingEnvironment {
    pub(crate) fn read_host_boot_id(procfs_root: &Path) -> io::Result<HostBootId> {
        let raw = fs::read_to_string(procfs_root.join("sys/kernel/random/boot_id"))?;
        raw.trim()
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error}")))
    }

    pub(crate) fn stale_reason_for_open_error(
        error: HostCgroupBindingError,
    ) -> ExternalBindingStaleReason {
        match error {
            HostCgroupBindingError::ContainerIdentityMissing
            | HostCgroupBindingError::ContainerIdentityMismatch
            | HostCgroupBindingError::Malformed(_) => {
                ExternalBindingStaleReason::ContainerIdentityMismatch
            }
            HostCgroupBindingError::RootMoved => ExternalBindingStaleReason::RootMoved,
            HostCgroupBindingError::Io { source, .. }
                if source.kind() == io::ErrorKind::PermissionDenied =>
            {
                ExternalBindingStaleReason::PermissionDenied
            }
            HostCgroupBindingError::Io { .. } => ExternalBindingStaleReason::BoundaryMissing,
            HostCgroupBindingError::Unsupported(_) => {
                ExternalBindingStaleReason::RequiredCounterUnavailable
            }
            HostCgroupBindingError::ExpectedStartTimeMissing
            | HostCgroupBindingError::PidReuse
            | HostCgroupBindingError::MembershipUnstable => {
                ExternalBindingStaleReason::ContainerIdentityMismatch
            }
        }
    }
}

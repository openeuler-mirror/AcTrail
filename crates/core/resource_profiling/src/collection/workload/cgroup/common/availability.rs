use crate::ProfilingError;
use linux_cgroup::CgroupError;

pub(crate) struct CgroupAvailability;
impl CgroupAvailability {
    pub(crate) fn is_unavailable(error: &CgroupError) -> bool {
        match error {
            CgroupError::Unsupported(_) => true,
            CgroupError::Io { source, .. } => matches!(
                source.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::ReadOnlyFilesystem
            ),
            _ => false,
        }
    }
}

/// Discovery, preflight and launch-admission errors caused by a missing or
/// read-only hierarchy are "unavailable": the caller may fall back.
pub(crate) fn cgroup_error(code: &'static str) -> impl FnOnce(CgroupError) -> ProfilingError {
    move |error| {
        let unavailable = matches!(
            code,
            "cgroup_discovery" | "cgroup_preflight" | "cgroup_launch_admission"
        ) && CgroupAvailability::is_unavailable(&error);
        ProfilingError::new(code, error.to_string()).availability(unavailable)
    }
}

use super::process_verifier::ProcfsProcessVerifier;
use super::{
    CgroupDirectoryIdentity, ContainerCgroupIdentity, ExternalScopeBinding, HostCgroupBindingError,
};
use linux_cgroup::{CgroupV2CounterReader, UnifiedHierarchy};
use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::process::HostProcessCoordinates;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path};
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

pub struct ContainerCgroupResolver;
impl ContainerCgroupResolver {
    pub fn bind_host_container_cgroup(
        procfs_root: &Path,
        hierarchy: &UnifiedHierarchy,
        coordinates: &HostProcessCoordinates,
        binding_attempts: u32,
    ) -> Result<ExternalScopeBinding, HostCgroupBindingError> {
        ProcfsProcessVerifier::require_expected_start(coordinates)?;
        for _ in 0..binding_attempts {
            ProcfsProcessVerifier::require_start_time(procfs_root, coordinates)?;
            let first_raw =
                ProcfsProcessVerifier::read_proc_file(procfs_root, coordinates.pid, "cgroup")?;
            let first = ContainerCgroupIdentity::parse_container_cgroup_identity(&first_raw)?
                .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
            let binding = Self::open_external_boundary(hierarchy, first.clone())?;
            ProcfsProcessVerifier::require_start_time(procfs_root, coordinates)?;
            let second_raw =
                ProcfsProcessVerifier::read_proc_file(procfs_root, coordinates.pid, "cgroup")?;
            let second = ContainerCgroupIdentity::parse_container_cgroup_identity(&second_raw)?
                .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
            if first.unified_process_path != second.unified_process_path {
                continue;
            }
            if first.runtime != second.runtime || first.container_id != second.container_id {
                return Err(HostCgroupBindingError::ContainerIdentityMismatch);
            }
            if !ContainerCgroupIdentity::component_ancestor(
                &first.container_boundary_path,
                &second.unified_process_path,
            ) {
                return Err(HostCgroupBindingError::RootMoved);
            }
            return Ok(binding);
        }
        Err(HostCgroupBindingError::MembershipUnstable)
    }

    /// Reopens a persisted runtime-owned cgroup without trusting its stored path as
    /// identity proof. The path must still parse to the exact persisted runtime and
    /// full container ID before it is opened beneath the cgroup2 mount.
    pub fn reopen_host_container_cgroup(
        hierarchy: &UnifiedHierarchy,
        runtime: ContainerRuntime,
        container_id: NormalizedContainerId,
        relative_path: &Path,
    ) -> Result<ExternalScopeBinding, HostCgroupBindingError> {
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(HostCgroupBindingError::Malformed(
                "persisted cgroup path must be canonical and relative".to_string(),
            ));
        }
        let boundary_path = hierarchy.mount_root.join(relative_path);
        let identity = ContainerCgroupIdentity::parse_unified_container_path(&boundary_path)?
            .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
        if identity.container_boundary_path != boundary_path
            || identity.runtime != runtime
            || identity.container_id != container_id
        {
            return Err(HostCgroupBindingError::ContainerIdentityMismatch);
        }
        Self::open_external_boundary(hierarchy, identity)
    }
    fn open_external_boundary(
        hierarchy: &UnifiedHierarchy,
        identity: ContainerCgroupIdentity,
    ) -> Result<ExternalScopeBinding, HostCgroupBindingError> {
        let relative_path = identity
            .container_boundary_path
            .strip_prefix(&hierarchy.mount_root)
            .map_err(|_| {
                HostCgroupBindingError::Unsupported(format!(
                    "container boundary {} is outside cgroup2 mount root {}",
                    identity.container_boundary_path.display(),
                    hierarchy.mount_root.display()
                ))
            })?
            .to_path_buf();
        if relative_path.as_os_str().is_empty() {
            return Err(HostCgroupBindingError::Unsupported(
                "container boundary cannot be the cgroup2 mount root".to_string(),
            ));
        }
        let mount = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&hierarchy.mount_point)
            .map_err(|source| HostCgroupBindingError::Io {
                operation: "open cgroup2 mount",
                path: hierarchy.mount_point.clone(),
                source,
            })?;
        let directory = Self::openat2_directory(&mount, &relative_path, &hierarchy.mount_point)?;
        let metadata = directory
            .metadata()
            .map_err(|source| HostCgroupBindingError::Io {
                operation: "stat container cgroup",
                path: hierarchy.mount_point.join(&relative_path),
                source,
            })?;
        let directory_identity = CgroupDirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let reader = CgroupV2CounterReader::from_open_directory(
            directory,
            hierarchy.mount_point.join(&relative_path),
        )
        .map_err(|error| HostCgroupBindingError::Unsupported(error.to_string()))?;
        Ok(ExternalScopeBinding {
            identity,
            relative_path,
            directory_identity,
            reader,
        })
    }

    fn openat2_directory(
        parent: &File,
        relative_path: &Path,
        display_root: &Path,
    ) -> Result<File, HostCgroupBindingError> {
        use std::os::unix::ffi::OsStrExt;

        let encoded = CString::new(relative_path.as_os_str().as_bytes()).map_err(|_| {
            HostCgroupBindingError::Malformed("cgroup path contains a NUL byte".to_string())
        })?;
        let how = OpenHow {
            flags: (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
            mode: 0,
            resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
        };
        let descriptor = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                parent.as_raw_fd(),
                encoded.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            ) as libc::c_int
        };
        if descriptor < 0 {
            return Err(HostCgroupBindingError::Io {
                operation: "open container cgroup beneath mount",
                path: display_root.join(relative_path),
                source: io::Error::last_os_error(),
            });
        }
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

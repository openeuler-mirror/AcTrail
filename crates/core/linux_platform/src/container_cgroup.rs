//! Strict container cgroup identity and read-only host binding.

use std::ffi::CString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::process::HostProcessCoordinates;

use crate::cgroup_v2::{
    CgroupCounters, CgroupV2CounterReader, UnifiedHierarchy, parse_unified_process_cgroup,
};

const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerCgroupIdentity {
    pub runtime: ContainerRuntime,
    pub container_id: NormalizedContainerId,
    pub pod_uid: Option<String>,
    pub unified_process_path: PathBuf,
    pub container_boundary_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CgroupDirectoryIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug)]
pub struct ExternalCgroupV2 {
    pub identity: ContainerCgroupIdentity,
    pub relative_path: PathBuf,
    pub directory_identity: CgroupDirectoryIdentity,
    reader: CgroupV2CounterReader,
}

impl ExternalCgroupV2 {
    pub fn read_counters(&mut self) -> Result<CgroupCounters, crate::cgroup_v2::CgroupError> {
        self.reader.read_counters()
    }

    pub fn verify_membership(
        &self,
        procfs_root: &Path,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), HostCgroupBindingError> {
        require_expected_start(coordinates)?;
        require_start_time(procfs_root, coordinates)?;
        let raw = read_proc_file(procfs_root, coordinates.pid, "cgroup")?;
        let current = parse_container_cgroup_identity(&raw)?
            .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
        if current.container_id != self.identity.container_id
            || current.runtime != self.identity.runtime
        {
            return Err(HostCgroupBindingError::ContainerIdentityMismatch);
        }
        if !component_ancestor(
            &self.identity.container_boundary_path,
            &current.unified_process_path,
        ) {
            return Err(HostCgroupBindingError::RootMoved);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum HostCgroupBindingError {
    ExpectedStartTimeMissing,
    PidReuse,
    MembershipUnstable,
    ContainerIdentityMissing,
    ContainerIdentityMismatch,
    RootMoved,
    Malformed(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Unsupported(String),
}

impl fmt::Display for HostCgroupBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedStartTimeMissing => {
                formatter.write_str("expected non-zero process start time is required")
            }
            Self::PidReuse => formatter.write_str("process start time changed (PID reuse)"),
            Self::MembershipUnstable => {
                formatter.write_str("process cgroup membership changed twice while binding")
            }
            Self::ContainerIdentityMissing => {
                formatter.write_str("process is not in a recognized host container cgroup")
            }
            Self::ContainerIdentityMismatch => {
                formatter.write_str("container identity no longer matches the binding")
            }
            Self::RootMoved => formatter.write_str("process moved outside the bound cgroup"),
            Self::Malformed(detail) => write!(formatter, "malformed cgroup identity: {detail}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::Unsupported(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for HostCgroupBindingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn parse_container_cgroup_identity(
    cgroup_file: &str,
) -> Result<Option<ContainerCgroupIdentity>, HostCgroupBindingError> {
    let unified = parse_unified_process_cgroup(cgroup_file)
        .map_err(|error| HostCgroupBindingError::Malformed(error.to_string()))?;
    parse_unified_container_path(&unified)
}

pub fn parse_unified_container_path(
    path: &Path,
) -> Result<Option<ContainerCgroupIdentity>, HostCgroupBindingError> {
    let components = canonical_absolute_components(path)?;
    parse_host_layout(path, &components)
}

pub fn bind_host_container_cgroup(
    procfs_root: &Path,
    hierarchy: &UnifiedHierarchy,
    coordinates: &HostProcessCoordinates,
) -> Result<ExternalCgroupV2, HostCgroupBindingError> {
    require_expected_start(coordinates)?;
    for attempt in 0..2 {
        require_start_time(procfs_root, coordinates)?;
        let first_raw = read_proc_file(procfs_root, coordinates.pid, "cgroup")?;
        let first = parse_container_cgroup_identity(&first_raw)?
            .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
        let binding = open_external_boundary(hierarchy, first.clone())?;
        require_start_time(procfs_root, coordinates)?;
        let second_raw = read_proc_file(procfs_root, coordinates.pid, "cgroup")?;
        let second = parse_container_cgroup_identity(&second_raw)?
            .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
        if first.unified_process_path != second.unified_process_path {
            if attempt == 0 {
                continue;
            }
            return Err(HostCgroupBindingError::MembershipUnstable);
        }
        if first.runtime != second.runtime || first.container_id != second.container_id {
            return Err(HostCgroupBindingError::ContainerIdentityMismatch);
        }
        if !component_ancestor(&first.container_boundary_path, &second.unified_process_path) {
            return Err(HostCgroupBindingError::RootMoved);
        }
        return Ok(binding);
    }
    unreachable!("binding loop either returns or reports instability")
}

/// Reopens a persisted runtime-owned cgroup without trusting its stored path as
/// identity proof. The path must still parse to the exact persisted runtime and
/// full container ID before it is opened beneath the cgroup2 mount.
pub fn reopen_host_container_cgroup(
    hierarchy: &UnifiedHierarchy,
    runtime: ContainerRuntime,
    container_id: NormalizedContainerId,
    relative_path: &Path,
) -> Result<ExternalCgroupV2, HostCgroupBindingError> {
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
    let identity = parse_unified_container_path(&boundary_path)?
        .ok_or(HostCgroupBindingError::ContainerIdentityMissing)?;
    if identity.container_boundary_path != boundary_path
        || identity.runtime != runtime
        || identity.container_id != container_id
    {
        return Err(HostCgroupBindingError::ContainerIdentityMismatch);
    }
    open_external_boundary(hierarchy, identity)
}

fn parse_host_layout(
    path: &Path,
    components: &[&str],
) -> Result<Option<ContainerCgroupIdentity>, HostCgroupBindingError> {
    let mut pod_uid = None;
    let mut pod_boundary_index = None;
    for (index, component) in components.iter().enumerate() {
        if let Some(uid) = pod_uid_from_component(component) {
            pod_uid = Some(uid);
            pod_boundary_index = Some(index);
        }

        for (prefix, runtime) in [
            ("docker-", ContainerRuntime::Docker),
            ("cri-containerd-", ContainerRuntime::Containerd),
            ("crio-", ContainerRuntime::Crio),
            ("libpod-", ContainerRuntime::Podman),
        ] {
            if let Some(raw_id) = component
                .strip_suffix(".scope")
                .and_then(|scope| scope.strip_prefix(prefix))
            {
                let Some(container_id) = strict_id(raw_id)? else {
                    return Ok(None);
                };
                return Ok(Some(identity_at(
                    path,
                    components,
                    index,
                    runtime,
                    container_id,
                    pod_uid,
                )));
            }
        }

        if index > 0
            && matches!(components[index - 1], "docker" | "libpod")
            && let Some(container_id) = strict_id(component)?
        {
            let runtime = if components[index - 1] == "docker" {
                ContainerRuntime::Docker
            } else {
                ContainerRuntime::Podman
            };
            return Ok(Some(identity_at(
                path,
                components,
                index,
                runtime,
                container_id,
                pod_uid,
            )));
        }

        if pod_boundary_index.is_some_and(|pod_index| index == pod_index + 1)
            && let Some(container_id) = strict_id(component)?
        {
            return Ok(Some(identity_at(
                path,
                components,
                index,
                ContainerRuntime::K8s,
                container_id,
                pod_uid,
            )));
        }
    }
    Ok(None)
}

fn identity_at(
    process_path: &Path,
    components: &[&str],
    boundary_index: usize,
    runtime: ContainerRuntime,
    container_id: NormalizedContainerId,
    pod_uid: Option<String>,
) -> ContainerCgroupIdentity {
    let boundary = components[..=boundary_index]
        .iter()
        .fold(PathBuf::from("/"), |path, component| path.join(component));
    ContainerCgroupIdentity {
        runtime,
        container_id,
        pod_uid,
        unified_process_path: process_path.to_path_buf(),
        container_boundary_path: boundary,
    }
}

fn strict_id(raw: &str) -> Result<Option<NormalizedContainerId>, HostCgroupBindingError> {
    if raw.len() != NormalizedContainerId::HEX_LENGTH {
        return Ok(None);
    }
    NormalizedContainerId::from_lower_hex(raw)
        .map(Some)
        .map_err(HostCgroupBindingError::Malformed)
}

fn canonical_absolute_components(path: &Path) -> Result<Vec<&str>, HostCgroupBindingError> {
    if !path.is_absolute() {
        return Err(HostCgroupBindingError::Malformed(
            "unified cgroup path must be absolute".to_string(),
        ));
    }
    let mut output = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => output.push(value.to_str().ok_or_else(|| {
                HostCgroupBindingError::Malformed("non-UTF-8 cgroup component".to_string())
            })?),
            _ => {
                return Err(HostCgroupBindingError::Malformed(
                    "cgroup path is not canonical".to_string(),
                ));
            }
        }
    }
    Ok(output)
}

fn pod_uid_from_component(component: &str) -> Option<String> {
    let raw = if let Some(raw) = component.strip_prefix("pod") {
        raw
    } else if component.starts_with("kubepods") && component.ends_with(".slice") {
        let without_suffix = component.strip_suffix(".slice")?;
        &without_suffix[without_suffix.rfind("pod")? + 3..]
    } else {
        return None;
    };
    let normalized = raw.replace('_', "-");
    is_uuid(&normalized).then_some(normalized)
}

fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn component_ancestor(boundary: &Path, membership: &Path) -> bool {
    membership == boundary || membership.starts_with(boundary)
}

fn require_expected_start(
    coordinates: &HostProcessCoordinates,
) -> Result<(), HostCgroupBindingError> {
    if coordinates.start_time_ticks == 0 {
        Err(HostCgroupBindingError::ExpectedStartTimeMissing)
    } else {
        Ok(())
    }
}

fn require_start_time(
    procfs_root: &Path,
    coordinates: &HostProcessCoordinates,
) -> Result<(), HostCgroupBindingError> {
    let raw = read_proc_file(procfs_root, coordinates.pid, "stat")?;
    let actual = parse_proc_start_time(&raw)?;
    if actual == coordinates.start_time_ticks {
        Ok(())
    } else {
        Err(HostCgroupBindingError::PidReuse)
    }
}

fn parse_proc_start_time(raw: &str) -> Result<u64, HostCgroupBindingError> {
    let close = raw.rfind(')').ok_or_else(|| {
        HostCgroupBindingError::Malformed("proc stat is missing command terminator".to_string())
    })?;
    let tail = raw.get(close + 1..).ok_or_else(|| {
        HostCgroupBindingError::Malformed("proc stat command offset overflow".to_string())
    })?;
    tail.split_whitespace()
        .nth(19)
        .ok_or_else(|| {
            HostCgroupBindingError::Malformed("proc stat is missing start time".to_string())
        })?
        .parse::<u64>()
        .map_err(|error| HostCgroupBindingError::Malformed(format!("invalid start time: {error}")))
}

fn read_proc_file(
    procfs_root: &Path,
    pid: u32,
    name: &'static str,
) -> Result<String, HostCgroupBindingError> {
    let path = procfs_root.join(pid.to_string()).join(name);
    fs::read_to_string(&path).map_err(|source| HostCgroupBindingError::Io {
        operation: "read process identity",
        path,
        source,
    })
}

fn open_external_boundary(
    hierarchy: &UnifiedHierarchy,
    identity: ContainerCgroupIdentity,
) -> Result<ExternalCgroupV2, HostCgroupBindingError> {
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
    let directory = openat2_directory(&mount, &relative_path, &hierarchy.mount_point)?;
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
    Ok(ExternalCgroupV2 {
        identity,
        relative_path,
        directory_identity,
        reader,
    })
}

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const POD: &str = "12345678-1234-1234-1234-123456789abc";

    #[test]
    fn host_policy_accepts_only_known_full_id_layouts() {
        let cases = [
            (
                format!("0::/system.slice/docker-{ID}.scope/app.service\n"),
                ContainerRuntime::Docker,
                format!("/system.slice/docker-{ID}.scope"),
            ),
            (
                format!("0::/docker/{ID}\n"),
                ContainerRuntime::Docker,
                format!("/docker/{ID}"),
            ),
            (
                format!(
                    "0::/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-pod{}.slice/cri-containerd-{ID}.scope\n",
                    POD.replace('-', "_")
                ),
                ContainerRuntime::Containerd,
                format!(
                    "/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-pod{}.slice/cri-containerd-{ID}.scope",
                    POD.replace('-', "_")
                ),
            ),
            (
                format!("0::/kubepods/pod{POD}/crio-{ID}.scope\n"),
                ContainerRuntime::Crio,
                format!("/kubepods/pod{POD}/crio-{ID}.scope"),
            ),
            (
                format!("0::/libpod/{ID}\n"),
                ContainerRuntime::Podman,
                format!("/libpod/{ID}"),
            ),
            (
                format!("0::/kubepods/burstable/pod{POD}/{ID}\n"),
                ContainerRuntime::K8s,
                format!("/kubepods/burstable/pod{POD}/{ID}"),
            ),
            (
                format!("0::/kubepods/burstable/pod{POD}/{ID}/app.service\n"),
                ContainerRuntime::K8s,
                format!("/kubepods/burstable/pod{POD}/{ID}"),
            ),
        ];
        for (raw, runtime, boundary) in cases {
            let identity = parse_container_cgroup_identity(&raw).unwrap().unwrap();
            assert_eq!(identity.runtime, runtime);
            assert_eq!(identity.container_id.to_string(), ID);
            assert_eq!(identity.container_boundary_path, Path::new(&boundary));
        }
    }

    #[test]
    fn policies_reject_prefix_uppercase_arbitrary_and_guest_only_host_layouts() {
        let prefix = &ID[..12];
        for raw in [
            format!("0::/docker/{prefix}\n"),
            format!("0::/arbitrary/{ID}\n"),
            format!("0::/default/{ID}\n"),
            format!("0::/k8s.io/{ID}\n"),
        ] {
            assert!(
                parse_container_cgroup_identity(&raw).unwrap().is_none(),
                "host parser accepted {raw:?}"
            );
        }
        assert!(
            parse_container_cgroup_identity(&format!("0::/docker/{}\n", ID.to_uppercase()))
                .is_err()
        );
    }

    #[test]
    fn cgroup_v1_and_noncanonical_paths_are_rejected() {
        assert!(parse_container_cgroup_identity(&format!("5:memory:/docker/{ID}\n")).is_err());
        assert!(parse_unified_container_path(Path::new("/docker/../docker/id")).is_err());
    }

    #[test]
    fn external_binding_requires_start_time_and_opens_verified_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let procfs = temp.path().join("proc");
        let mount = temp.path().join("cgroup");
        fs::create_dir_all(procfs.join("42")).unwrap();
        let boundary = mount.join("docker").join(ID);
        let descendant = boundary.join("app.service");
        fs::create_dir_all(&descendant).unwrap();
        write_proc_stat(&procfs, 42, 900);
        fs::write(
            procfs.join("42/cgroup"),
            format!("0::/docker/{ID}/app.service\n"),
        )
        .unwrap();
        let hierarchy = UnifiedHierarchy {
            mount_point: mount,
            mount_root: PathBuf::from("/"),
            process_relative_path: PathBuf::from("/"),
            process_path: temp.path().join("cgroup"),
        };

        assert!(matches!(
            bind_host_container_cgroup(&procfs, &hierarchy, &HostProcessCoordinates::new(42, 0)),
            Err(HostCgroupBindingError::ExpectedStartTimeMissing)
        ));
        let binding =
            bind_host_container_cgroup(&procfs, &hierarchy, &HostProcessCoordinates::new(42, 900))
                .unwrap();
        assert_eq!(
            binding.identity.container_boundary_path,
            Path::new(&format!("/docker/{ID}"))
        );
        assert_eq!(binding.relative_path, Path::new(&format!("docker/{ID}")));
        assert_eq!(
            binding.directory_identity.inode,
            fs::metadata(boundary).unwrap().ino()
        );
    }

    #[test]
    fn membership_verification_detects_pid_reuse_and_root_movement() {
        let temp = tempfile::tempdir().unwrap();
        let procfs = temp.path().join("proc");
        let mount = temp.path().join("cgroup");
        fs::create_dir_all(procfs.join("42")).unwrap();
        fs::create_dir_all(mount.join("docker").join(ID)).unwrap();
        write_proc_stat(&procfs, 42, 900);
        fs::write(procfs.join("42/cgroup"), format!("0::/docker/{ID}\n")).unwrap();
        let hierarchy = UnifiedHierarchy {
            mount_point: mount,
            mount_root: PathBuf::from("/"),
            process_relative_path: PathBuf::from("/"),
            process_path: temp.path().join("cgroup"),
        };
        let coordinates = HostProcessCoordinates::new(42, 900);
        let binding = bind_host_container_cgroup(&procfs, &hierarchy, &coordinates).unwrap();

        write_proc_stat(&procfs, 42, 901);
        assert!(matches!(
            binding.verify_membership(&procfs, &coordinates),
            Err(HostCgroupBindingError::PidReuse)
        ));
        write_proc_stat(&procfs, 42, 900);
        let other = "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd";
        fs::write(procfs.join("42/cgroup"), format!("0::/docker/{other}\n")).unwrap();
        assert!(matches!(
            binding.verify_membership(&procfs, &coordinates),
            Err(HostCgroupBindingError::ContainerIdentityMismatch)
        ));
    }

    #[test]
    fn persisted_binding_reopen_revalidates_path_runtime_and_full_id() {
        let temp = tempfile::tempdir().unwrap();
        let mount = temp.path().join("cgroup");
        let relative = PathBuf::from(format!("docker/{ID}"));
        let boundary = mount.join(&relative);
        fs::create_dir_all(&boundary).unwrap();
        let hierarchy = UnifiedHierarchy {
            mount_point: mount,
            mount_root: PathBuf::from("/"),
            process_relative_path: PathBuf::from("/"),
            process_path: temp.path().join("cgroup"),
        };
        let container_id = NormalizedContainerId::from_lower_hex(ID).unwrap();
        let reopened = reopen_host_container_cgroup(
            &hierarchy,
            ContainerRuntime::Docker,
            container_id,
            &relative,
        )
        .unwrap();
        assert_eq!(
            reopened.directory_identity.inode,
            fs::metadata(&boundary).unwrap().ino()
        );
        assert!(matches!(
            reopen_host_container_cgroup(
                &hierarchy,
                ContainerRuntime::Podman,
                container_id,
                &relative,
            ),
            Err(HostCgroupBindingError::ContainerIdentityMismatch)
        ));
        assert!(matches!(
            reopen_host_container_cgroup(
                &hierarchy,
                ContainerRuntime::Docker,
                container_id,
                Path::new("../docker/id"),
            ),
            Err(HostCgroupBindingError::Malformed(_))
        ));
    }

    fn write_proc_stat(procfs: &Path, pid: u32, start_time: u64) {
        let mut fields = vec!["0".to_string(); 19];
        fields[0] = "S".to_string();
        fields.push(start_time.to_string());
        fs::write(
            procfs.join(pid.to_string()).join("stat"),
            format!("{pid} (test command) {}\n", fields.join(" ")),
        )
        .unwrap();
    }
}

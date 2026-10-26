use super::*;
use linux_cgroup::UnifiedHierarchy;
use model_core::container::NormalizedContainerId;
use model_core::{container::ContainerRuntime, process::HostProcessCoordinates};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::{fs, path::PathBuf};

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
        let identity = ContainerCgroupIdentity::parse_container_cgroup_identity(&raw)
            .unwrap()
            .unwrap();
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
            ContainerCgroupIdentity::parse_container_cgroup_identity(&raw)
                .unwrap()
                .is_none(),
            "host parser accepted {raw:?}"
        );
    }
    assert!(
        ContainerCgroupIdentity::parse_container_cgroup_identity(&format!(
            "0::/docker/{}\n",
            ID.to_uppercase()
        ))
        .is_err()
    );
}

#[test]
fn cgroup_v1_and_noncanonical_paths_are_rejected() {
    assert!(
        ContainerCgroupIdentity::parse_container_cgroup_identity(&format!(
            "5:memory:/docker/{ID}\n"
        ))
        .is_err()
    );
    assert!(
        ContainerCgroupIdentity::parse_unified_container_path(Path::new("/docker/../docker/id"))
            .is_err()
    );
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
        ContainerCgroupResolver::bind_host_container_cgroup(
            &procfs,
            &hierarchy,
            &HostProcessCoordinates::new(42, 0),
            2,
        ),
        Err(HostCgroupBindingError::ExpectedStartTimeMissing)
    ));
    let binding = ContainerCgroupResolver::bind_host_container_cgroup(
        &procfs,
        &hierarchy,
        &HostProcessCoordinates::new(42, 900),
        2,
    )
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
    let binding =
        ContainerCgroupResolver::bind_host_container_cgroup(&procfs, &hierarchy, &coordinates, 2)
            .unwrap();

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
    let reopened = ContainerCgroupResolver::reopen_host_container_cgroup(
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
        ContainerCgroupResolver::reopen_host_container_cgroup(
            &hierarchy,
            ContainerRuntime::Podman,
            container_id,
            &relative,
        ),
        Err(HostCgroupBindingError::ContainerIdentityMismatch)
    ));
    assert!(matches!(
        ContainerCgroupResolver::reopen_host_container_cgroup(
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

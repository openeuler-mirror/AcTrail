//! Existing procfs behavior tests.

use process_identity::ProcessIdentityReader;
use process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter;

use std::path::PathBuf;

use model_core::container::ContainerRuntime;
use model_core::process::NamespaceIdentity;

use super::{
    ProcfsIdentityReader, ProcfsTreeSnapshotter, host_path_via_anchor, parse_container_identity,
    read_pid_namespace, resolve_host_path,
};

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn parse_container_docker_v2_systemd() {
    let cgroup = format!("0::/system.slice/docker-{ID}.scope\n");
    let identity = parse_container_identity(&cgroup).expect("docker id");
    assert_eq!(identity.runtime, ContainerRuntime::Docker);
    assert_eq!(identity.container_id, ID);
    assert!(identity.pod_uid.is_none());
}

#[test]
fn parse_container_docker_v1_cgroupfs() {
    let cgroup = format!("12:pids:/docker/{ID}\n11:memory:/docker/{ID}\n");
    let identity = parse_container_identity(&cgroup).expect("docker id");
    assert_eq!(identity.runtime, ContainerRuntime::Docker);
    assert_eq!(identity.container_id, ID);
}

const POD_UID: &str = "2ee7d8a2-e832-4a13-b26c-02ad9ae4a8f6";

#[test]
fn parse_container_cri_containerd_systemd_with_pod_uid() {
    let uid_underscored = POD_UID.replace('-', "_");
    let cgroup = format!(
        "0::/kubepods.slice/kubepods-besteffort.slice/\
         kubepods-besteffort-pod{uid_underscored}.slice/cri-containerd-{ID}.scope\n"
    );
    let identity = parse_container_identity(&cgroup).expect("containerd id");
    assert_eq!(identity.runtime, ContainerRuntime::Containerd);
    assert_eq!(identity.container_id, ID);
    assert_eq!(identity.pod_uid.as_deref(), Some(POD_UID));
}

#[test]
fn unsupported_runtime_layouts_do_not_resolve_identity() {
    let uid_underscored = POD_UID.replace('-', "_");
    let crio = format!(
        "0::/kubepods.slice/kubepods-burstable.slice/\
         kubepods-burstable-pod{uid_underscored}.slice/crio-{ID}.scope\n"
    );
    let scoped = format!("0::/machine.slice/libpod-{ID}.scope\n");
    let cgroupfs = format!("12:pids:/libpod_parent/libpod-{ID}\n");

    assert!(parse_container_identity(&crio).is_none());
    assert!(parse_container_identity(&scoped).is_none());
    assert!(parse_container_identity(&cgroupfs).is_none());
}

#[test]
fn parse_container_kubepods_cgroupfs_leaf() {
    let cgroup = format!("12:pids:/kubepods/besteffort/pod{POD_UID}/{ID}\n");
    let identity = parse_container_identity(&cgroup).expect("cri leaf id");
    assert_eq!(identity.runtime, ContainerRuntime::K8s);
    assert_eq!(identity.container_id, ID);
    assert_eq!(identity.pod_uid.as_deref(), Some(POD_UID));
}

#[test]
fn parse_container_pod_uid_is_never_the_container_id() {
    // A pod directory without a container leaf must not yield an identity,
    // and the pod UID must never be reported as a container id.
    let cgroup = format!("0::/kubepods/besteffort/pod{POD_UID}\n");
    assert!(parse_container_identity(&cgroup).is_none());

    let uid_underscored = POD_UID.replace('-', "_");
    let systemd_only_pod = format!(
        "0::/kubepods.slice/kubepods-besteffort.slice/\
         kubepods-besteffort-pod{uid_underscored}.slice\n"
    );
    assert!(parse_container_identity(&systemd_only_pod).is_none());
}

#[test]
fn parse_container_kata_agent_guest_layout() {
    // Real fixture captured from a Kata guest via debug console: kata-agent
    // lays workload cgroups out cgroup-v2 cgroupfs-style as
    // `/<containerd-namespace>/<container-id>`. The guest daemon reads this
    // from the guest root namespace (a container-internal read is masked to
    // `0::/` by the cgroup namespace, so it must be read guest-side).
    let k8s = format!("0::/k8s.io/{ID}\n");
    let identity = parse_container_identity(&k8s).expect("kata k8s.io leaf");
    assert_eq!(identity.runtime, ContainerRuntime::Containerd);
    assert_eq!(identity.container_id, ID);

    let default_ns = format!("0::/default/{ID}\n");
    let identity = parse_container_identity(&default_ns).expect("kata default leaf");
    assert_eq!(identity.runtime, ContainerRuntime::Containerd);
    assert_eq!(identity.container_id, ID);
}

#[test]
fn parse_container_kata_agent_guest_v1_layout() {
    let cgroup = format!(
        "12:pids:/k8s.io/{ID}\n\
         11:memory:/k8s.io/{ID}\n\
         10:devices:/k8s.io/{ID}\n"
    );
    let identity = parse_container_identity(&cgroup).expect("kata v1 k8s.io leaf");
    assert_eq!(identity.runtime, ContainerRuntime::Containerd);
    assert_eq!(identity.container_id, ID);

    let default_ns = format!(
        "12:pids:/default/{ID}\n\
         11:memory:/default/{ID}\n"
    );
    let identity = parse_container_identity(&default_ns).expect("kata v1 default leaf");
    assert_eq!(identity.runtime, ContainerRuntime::Containerd);
    assert_eq!(identity.container_id, ID);
}

#[test]
fn parse_container_kata_guest_system_paths_are_none() {
    // Guest system services and init must NOT be mistaken for containers
    // (their leaves are not container-id shaped).
    assert!(parse_container_identity("0::/init.scope\n").is_none());
    assert!(parse_container_identity("0::/system.slice/kata-agent.service\n").is_none());
    assert!(parse_container_identity("0::/system.slice/systemd-journald.service\n").is_none());
}

#[test]
fn parse_container_host_is_none() {
    let cgroup = "0::/user.slice/user-1000.slice/session-3.scope\n";
    assert!(parse_container_identity(cgroup).is_none());
}

#[test]
fn containerized_heuristic_flags_unparsed_layouts_but_not_host() {
    use super::cgroup_looks_containerized;
    // Unmapped guest layouts (kata-agent style) must trip the backstop…
    assert!(cgroup_looks_containerized("0::/vc/abc123\n"));
    assert!(cgroup_looks_containerized("0::/kata_sandbox/agent\n"));
    assert!(cgroup_looks_containerized("0::/default/guestprobe\n"));
    assert!(cgroup_looks_containerized("0::/k8s.io/guestprobe\n"));
    assert!(cgroup_looks_containerized(&format!(
        "0::/kubepods/besteffort/pod{POD_UID}\n"
    )));
    assert!(parse_container_identity("0::/default/guestprobe\n").is_none());
    assert!(parse_container_identity("0::/k8s.io/guestprobe\n").is_none());
    // …while plain host paths stay clean.
    assert!(!cgroup_looks_containerized(
        "0::/user.slice/user-1000.slice/session-3.scope\n"
    ));
    assert!(!cgroup_looks_containerized("0::/init.scope\n"));
    assert!(!cgroup_looks_containerized("0::/default\n"));
    assert!(!cgroup_looks_containerized(""));
}

#[test]
fn parse_container_garbage_is_none() {
    assert!(parse_container_identity("").is_none());
    assert!(parse_container_identity("no colons here\n").is_none());
    assert!(parse_container_identity("0::/system.slice/docker-short.scope\n").is_none());
}

#[test]
fn host_path_via_anchor_strips_leading_slash() {
    assert_eq!(
        host_path_via_anchor(42, "/app/data.txt"),
        PathBuf::from("/proc/42/root/app/data.txt")
    );
    assert_eq!(
        host_path_via_anchor(7, "/a/b/c"),
        PathBuf::from("/proc/7/root/a/b/c")
    );
}

#[test]
fn resolve_host_path_rejects_relative_path() {
    let ns = NamespaceIdentity::new("pid:[4026531836]");
    assert!(resolve_host_path(&ns, "relative/path").is_none());
}

#[test]
fn resolve_host_path_rejects_unknown_namespace() {
    let ns = NamespaceIdentity::new("unknown");
    assert!(resolve_host_path(&ns, "/app/data.txt").is_none());
}

#[test]
fn resolve_host_path_maps_through_live_anchor() {
    // The current process is a live member of its own pid namespace, so it
    // is always a valid anchor when namespaces are visible.
    let ns = read_pid_namespace(std::process::id());
    if ns.as_str() == "unknown" {
        return; // no namespace visibility in this environment
    }
    let mapped = resolve_host_path(&ns, "/etc/hostname").expect("live anchor exists");
    let rendered = mapped.to_string_lossy();
    assert!(rendered.starts_with("/proc/"));
    assert!(rendered.ends_with("/root/etc/hostname"));
}

#[test]
fn identity_reader_reads_current_process() {
    let identity = ProcfsIdentityReader
        .read_identity(std::process::id())
        .unwrap();
    let host = identity.host.expect("host coordinates");
    assert_eq!(host.pid, std::process::id());
    assert!(host.start_time_ticks > 0);
}

#[test]
fn tree_snapshot_contains_root_process() {
    let identity = ProcfsIdentityReader
        .read_identity(std::process::id())
        .unwrap();
    let snapshot = ProcfsTreeSnapshotter.snapshot(&identity).unwrap();
    assert!(snapshot.processes.iter().any(|process| {
        process
            .identity
            .host
            .as_ref()
            .is_some_and(|host| host.pid == std::process::id())
    }));
}

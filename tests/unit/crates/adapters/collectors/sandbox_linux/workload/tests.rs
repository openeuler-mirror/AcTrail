// Tests for the corresponding production module.
use super::*;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn fixture(root: &Path) -> WorkloadCgroupCollector {
    let procfs = root.join("proc");
    fs::create_dir_all(&procfs).unwrap();
    WorkloadCgroupCollector {
        procfs: ProcfsReader::open(procfs).unwrap(),
        boot_id: GuestBootId::new([1; 16]),
        mount_point: root.join("cg"),
        mount_root: "/".into(),
        cgroup2_mount_id: 7,
        root_process_names: vec![marker(1).executable_name],
    }
}

fn marker(pid: u32) -> ProcessMarker {
    let mut name = [0; 16];
    name[..4].copy_from_slice(b"root");
    ProcessMarker {
        pid,
        start_time_ticks: 900,
        executable_name: name,
    }
}

fn write_root(collector: &WorkloadCgroupCollector, pid: u32, cgroup: Option<&str>) {
    let root = collector.procfs.root().join(pid.to_string());
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("comm"), "root\n").unwrap();
    let mut fields = vec!["0"; 20];
    fields[0] = "S";
    fields[19] = "900";
    fs::write(
        root.join("stat"),
        format!("{pid} (root) {}", fields.join(" ")),
    )
    .unwrap();
    if let Some(cgroup) = cgroup {
        fs::write(root.join("cgroup"), cgroup).unwrap();
    }
}

fn write_counters(path: &Path, memory: u64) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("memory.current"), memory.to_string()).unwrap();
    fs::write(path.join("cpu.stat"), "usage_usec 100\n").unwrap();
}

#[test]
fn replacement_after_open_cannot_mix_directory_identity_and_counters() {
    let temp = tempfile::tempdir().unwrap();
    let collector = fixture(temp.path());
    let relative = format!("/default/{ID}");
    let boundary = parse_guest_boundary(Path::new(&relative)).unwrap().unwrap();
    let path = collector.mount_point.join(format!("default/{ID}"));
    write_counters(&path, 1024);
    let directory = fs::File::open(&path).unwrap();
    let metadata = directory.metadata().unwrap();
    // Deterministically replace the pathname between open and identity/read.
    fs::rename(&path, path.with_file_name("old-boundary")).unwrap();
    write_counters(&path, 8192);
    let sample = collector
        .read_open_workload(&boundary, &[marker(42)], 1, directory, &path)
        .unwrap();
    assert_eq!(sample.counters.memory_current_bytes, 1024);
    assert_eq!(
        sample.workload_id,
        WorkloadCgroupId::compute(
            &collector.boot_id,
            7,
            metadata.dev(),
            metadata.ino(),
            relative.as_bytes()
        )
        .unwrap()
    );
    let replacement = collector
        .read_workload(&boundary, &[marker(42)], 2)
        .unwrap();
    assert_eq!(replacement.counters.memory_current_bytes, 8192);
    assert_ne!(sample.workload_id, replacement.workload_id);
}

#[test]
fn missing_unreadable_and_malformed_roots_do_not_suppress_healthy_workloads() {
    let temp = tempfile::tempdir().unwrap();
    let collector = fixture(temp.path());
    write_root(&collector, 1, None);
    write_root(&collector, 2, Some("0::/default/not-a-container\n"));
    write_root(&collector, 3, None);
    // A directory deterministically fails read_to_string even when tests run as root.
    fs::create_dir(collector.procfs.root().join("3/cgroup")).unwrap();
    write_root(&collector, 4, Some(&format!("0::/default/{ID}\n")));
    write_root(&collector, 5, Some(&format!("0::/default/{ID}/child\n")));
    write_counters(&collector.mount_point.join(format!("default/{ID}")), 4096);
    let snapshots = collector.sample().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].counters.memory_current_bytes, 4096);
    assert_eq!(snapshots[0].monitored_root_count, 2);
}

#[test]
fn guest_boundary_accepts_default_and_k8s_io_namespaces() {
    for namespace in ["default", "k8s.io"] {
        let boundary = parse_guest_boundary(Path::new(&format!("/{namespace}/{ID}")))
            .unwrap()
            .unwrap();
        assert_eq!(boundary.runtime, SandboxContainerRuntime::Containerd);
        assert_eq!(boundary.container_id.to_lower_hex(), ID);
        assert_eq!(
            boundary.boundary_relative_path,
            Path::new(&format!("/{namespace}/{ID}"))
        );
    }
}

#[test]
fn guest_boundary_uses_container_prefix_for_descendant_paths() {
    let boundary = parse_guest_boundary(Path::new(&format!("/default/{ID}/app.service")))
        .unwrap()
        .unwrap();
    assert_eq!(
        boundary.boundary_relative_path,
        Path::new(&format!("/default/{ID}"))
    );
}

#[test]
fn guest_boundary_rejects_non_guest_layouts() {
    for path in [
        format!("/docker/{ID}"),
        "/system.slice/foo".to_string(),
        format!("/arbitrary/{ID}"),
    ] {
        assert!(
            parse_guest_boundary(Path::new(&path)).unwrap().is_none(),
            "path {path:?} should not be a guest boundary"
        );
    }
}

#[test]
fn guest_boundary_rejects_non_hex_container_ids() {
    assert!(parse_guest_boundary(Path::new("/default/not-hex")).is_err());
    assert!(parse_guest_boundary(Path::new(&format!("/default/{}", ID.to_uppercase()))).is_err());
}

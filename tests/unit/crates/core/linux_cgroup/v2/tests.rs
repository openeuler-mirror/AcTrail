// Tests for the corresponding production module.
use super::*;

#[test]
fn managed_paths_reject_traversal_and_outside_scopes() {
    assert!(ManagedCgroupV2::new("relative/root").is_err());
    assert!(ManagedCgroupV2::new("/").is_err());

    let adapter = ManagedCgroupV2::new("/sys/fs/cgroup/actrail").unwrap();
    assert!(adapter.trace_paths(1, "../escape").is_err());
    assert!(adapter.trace_paths(1, "").is_err());
    assert!(
        adapter
            .verify_descendant(Path::new("/sys/fs/cgroup/other"))
            .is_err()
    );
}

#[test]
fn unified_hierarchy_discovery_handles_mount_roots_and_escapes() {
    let hierarchy = discover_unified_hierarchy_from(
        "29 23 0:26 /delegated /sys/fs/cgroup rw,nosuid,nodev - cgroup2 cgroup rw\n",
        "0::/delegated/service.slice/actraild.service\n",
    )
    .unwrap();
    assert_eq!(hierarchy.mount_point, Path::new("/sys/fs/cgroup"));
    assert_eq!(hierarchy.mount_root, Path::new("/delegated"));
    assert_eq!(
        hierarchy.process_path,
        Path::new("/sys/fs/cgroup/service.slice/actraild.service")
    );

    let (root, point) =
        parse_cgroup2_mount("29 23 0:26 / /sys/fs/cgroup\\040v2 rw - cgroup2 cgroup rw\n").unwrap();
    assert_eq!(root, Path::new("/"));
    assert_eq!(point, Path::new("/sys/fs/cgroup v2"));
}

#[test]
fn unified_hierarchy_discovery_rejects_v1_and_outside_mount_root() {
    assert!(
        discover_unified_hierarchy_from(
            "29 23 0:26 / /sys/fs/cgroup rw - cgroup cgroup rw\n",
            "2:cpu:/service\n"
        )
        .is_err()
    );
    assert!(
        resolve_process_cgroup_path(
            Path::new("/sys/fs/cgroup"),
            Path::new("/delegated"),
            Path::new("/other/service")
        )
        .is_err()
    );
}

#[test]
fn key_value_parsers_tolerate_order_and_preserve_unknown_keys() {
    let events = parse_memory_events(
        "oom_kill 4\nfuture_counter 99\nlow 1\noom_group_kill 5\nhigh 2\nmax 3\noom 6\n",
        "memory.events",
    )
    .unwrap();
    assert_eq!(events.low, Some(1));
    assert_eq!(events.oom_kill, Some(4));
    assert_eq!(
        events.unknown.get("future_counter"),
        Some(&"99".to_string())
    );

    let cpu = parse_cpu_stat(
        "nr_throttled 8\nusage_usec 100\nfuture_cpu value\nuser_usec 60\nsystem_usec 40\n",
    )
    .unwrap();
    assert_eq!(cpu.usage_usec, 100);
    assert_eq!(cpu.user_usec, Some(60));
    assert_eq!(cpu.unknown.get("future_cpu"), Some(&"value".to_string()));
}

#[test]
fn known_duplicate_and_invalid_counters_fail() {
    assert!(matches!(
        parse_cpu_stat("usage_usec 1\nusage_usec 2\n"),
        Err(CgroupError::Malformed { .. })
    ));
    assert!(matches!(
        parse_memory_events("oom nope\n", "memory.events"),
        Err(CgroupError::Malformed { .. })
    ));
    assert!(matches!(
        parse_populated("populated 2\n"),
        Err(CgroupError::Malformed { .. })
    ));
}

#[test]
fn io_stat_aggregates_devices_and_diagnoses_overflow() {
    let io =
        parse_io_stat("8:0 rbytes=10 wbytes=20 rios=1\n8:16 wbytes=5 rbytes=7 future=3\n").unwrap();
    assert_eq!(io.read_bytes, Some(17));
    assert_eq!(io.write_bytes, Some(25));
    assert_eq!(io.unknown.get("8:16.future"), Some(&"3".to_string()));
    assert!(matches!(
        parse_io_stat(&format!("8:0 rbytes={}\n8:1 rbytes=1\n", u64::MAX)),
        Err(CgroupError::CounterOverflow { .. })
    ));
}

#[test]
fn fake_filesystem_scope_reads_required_and_optional_counters() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("actrail");
    fs::create_dir(&root).unwrap();
    let adapter = ManagedCgroupV2::new(&root).unwrap();
    adapter.create_managed_hierarchy().unwrap();
    let paths = adapter.trace_paths(17, "abc123").unwrap();
    adapter.create_trace_scope(&paths).unwrap();

    write(&paths.aggregate, "memory.current", "4096\n");
    write(&paths.aggregate, "memory.peak", "8192\n");
    write(&paths.aggregate, "memory.stat", "file 1024\nanon 3072\n");
    write(
        &paths.aggregate,
        "memory.events",
        "low 0\nhigh 1\nmax 2\noom 3\noom_kill 4\n",
    );
    write(
        &paths.aggregate,
        "cpu.stat",
        "system_usec 40\nusage_usec 100\nuser_usec 60\n",
    );
    write(&paths.aggregate, "io.stat", "8:0 rbytes=12 wbytes=34\n");
    write(&paths.aggregate, "pids.current", "2\n");
    write(&paths.aggregate, "cgroup.events", "populated 0\nfrozen 0\n");

    let counters = adapter.read_counters(&paths.aggregate).unwrap();
    assert_eq!(counters.memory_current_bytes, 4096);
    assert_eq!(counters.memory_peak_bytes, Some(8192));
    assert_eq!(counters.memory_anon_bytes, Some(3072));
    assert_eq!(counters.memory_swap_current_bytes, None);
    assert_eq!(counters.cpu.usage_usec, 100);
    assert_eq!(counters.io.unwrap().write_bytes, Some(34));
    assert_eq!(counters.pids_current, Some(2));
    assert_eq!(counters.pids_peak, None);
    assert!(!adapter.read_populated(&paths.aggregate).unwrap());

    let empty_paths = adapter.trace_paths(18, "cleanup").unwrap();
    adapter.create_trace_scope(&empty_paths).unwrap();
    adapter.remove_empty_trace_scope(&empty_paths).unwrap();
}

#[test]
fn counter_reader_keeps_memory_peak_descriptor_read_only_and_stable() {
    let temp = tempfile::tempdir().unwrap();
    let aggregate = temp.path().join("trace");
    fs::create_dir(&aggregate).unwrap();
    write(&aggregate, "memory.current", "4096\n");
    write(&aggregate, "memory.peak", "8192\n");
    write(&aggregate, "cpu.stat", "usage_usec 100\n");

    let mut reader = CgroupV2CounterReader::open(&aggregate).unwrap();
    assert_eq!(
        reader.read_counters().unwrap().memory_peak_bytes,
        Some(8192)
    );

    let OptionalCounterDescriptor::Open(memory_peak) = &reader.memory_peak else {
        panic!("memory.peak descriptor was not retained");
    };
    let status_flags = unsafe { libc::fcntl(memory_peak.as_raw_fd(), libc::F_GETFL) };
    assert!(status_flags >= 0);
    assert_eq!(status_flags & libc::O_ACCMODE, libc::O_RDONLY);
    let descriptor_flags = unsafe { libc::fcntl(memory_peak.as_raw_fd(), libc::F_GETFD) };
    assert!(descriptor_flags >= 0);
    assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0);

    fs::rename(
        aggregate.join("memory.peak"),
        aggregate.join("memory.peak.original"),
    )
    .unwrap();
    write(&aggregate, "memory.peak", "16384\n");
    assert_eq!(
        reader.read_counters().unwrap().memory_peak_bytes,
        Some(8192),
        "the reader must retain its original read-only peak descriptor"
    );
}

#[test]
fn counter_reader_rejects_symlinked_directory_and_counter() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let aggregate = temp.path().join("trace");
    fs::create_dir(&aggregate).unwrap();
    write(&aggregate, "memory.current", "4096\n");
    write(&aggregate, "cpu.stat", "usage_usec 100\n");
    let outside_peak = temp.path().join("outside-memory.peak");
    fs::write(&outside_peak, "8192\n").unwrap();
    symlink(&outside_peak, aggregate.join("memory.peak")).unwrap();

    let linked_directory = temp.path().join("linked-trace");
    symlink(&aggregate, &linked_directory).unwrap();
    assert!(CgroupV2CounterReader::open(&linked_directory).is_err());

    let mut reader = CgroupV2CounterReader::open(&aggregate).unwrap();
    assert!(reader.read_counters().is_err());
}

#[test]
fn managed_scope_scan_is_bounded_and_separates_unknown_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("actrail");
    fs::create_dir(&root).unwrap();
    let adapter = ManagedCgroupV2::new(&root).unwrap();
    adapter.create_managed_hierarchy().unwrap();
    let valid = adapter.trace_paths(4, "known-nonce").unwrap();
    adapter.create_trace_scope(&valid).unwrap();
    fs::create_dir(root.join("traces/not-managed")).unwrap();
    fs::write(root.join("traces/cgroup.controllers"), "cpu memory\n").unwrap();

    let scan = adapter.scan_managed_scopes(2).unwrap();
    assert_eq!(scan.managed.len(), 1);
    assert_eq!(scan.managed[0].trace_id, 4);
    assert_eq!(scan.managed[0].nonce, "known-nonce");
    assert_eq!(scan.unknown, vec![root.join("traces/not-managed")]);
    assert!(adapter.scan_managed_scopes(1).is_err());
}

#[test]
fn recursive_empty_check_observes_descendant_processes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("actrail");
    fs::create_dir(&root).unwrap();
    let adapter = ManagedCgroupV2::new(&root).unwrap();
    adapter.create_managed_hierarchy().unwrap();
    let paths = adapter.trace_paths(5, "emptycheck").unwrap();
    adapter.create_trace_scope(&paths).unwrap();
    write(&paths.aggregate, "cgroup.procs", "");
    write(&paths.workload, "cgroup.procs", "123\n456\n");
    assert!(!adapter.subtree_is_empty(&paths.aggregate).unwrap());
    assert_eq!(
        adapter.read_subtree_pids(&paths.aggregate).unwrap(),
        vec![123, 456]
    );

    write(&paths.workload, "cgroup.procs", "");
    assert!(adapter.subtree_is_empty(&paths.aggregate).unwrap());
}

fn write(directory: &Path, name: &str, value: &str) {
    fs::write(directory.join(name), value).unwrap();
}

#[test]
fn cleanup_removes_nested_scopes_and_is_idempotent_after_partial_progress() {
    let temp = tempfile::tempdir().unwrap();
    let adapter = ManagedCgroupV2::new(temp.path().join("cg")).unwrap();
    fs::create_dir(adapter.root()).unwrap();
    adapter.create_managed_hierarchy().unwrap();
    let paths = adapter.trace_paths(1, "nested").unwrap();
    adapter.create_trace_scope(&paths).unwrap();
    fs::create_dir_all(paths.workload.join("child/grandchild")).unwrap();
    fs::create_dir_all(paths.aggregate.join("sibling/leaf")).unwrap();
    // A regular file blocks rmdir in the fixture, simulating a failed
    // parent removal after all child directories have already disappeared.
    fs::write(paths.aggregate.join("blocker"), "").unwrap();
    assert!(adapter.remove_empty_trace_scope(&paths).is_err());
    assert!(!paths.workload.exists());
    let scan = adapter.scan_managed_scopes(10).unwrap();
    assert!(scan.unknown.is_empty());
    assert_eq!(scan.managed[0].paths, paths);
    fs::remove_file(paths.aggregate.join("blocker")).unwrap();
    adapter.remove_empty_trace_scope(&paths).unwrap();
    adapter.remove_empty_trace_scope(&paths).unwrap();
    assert!(!paths.aggregate.exists());
    assert!(adapter.subtree_is_empty(&paths.aggregate).unwrap());
}

#[test]
fn cleanup_never_follows_symlinked_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let adapter = ManagedCgroupV2::new(temp.path().join("cg")).unwrap();
    fs::create_dir(adapter.root()).unwrap();
    adapter.create_managed_hierarchy().unwrap();
    let paths = adapter.trace_paths(1, "link").unwrap();
    adapter.create_trace_scope(&paths).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, paths.workload.join("link")).unwrap();
    assert!(adapter.remove_empty_trace_scope(&paths).is_err());
    assert!(outside.is_dir());
}

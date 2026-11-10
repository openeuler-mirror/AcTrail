//! Drives the backend on a real Btrfs filesystem that tests/run-real.sh prepares as root.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::Command;

use abi_stable::std_types::RVec;
use actrail_ckpt_fs_btrfs::BtrfsBackend;
use checkpoint::{
    BackendError, BackendErrorKind, RestorableCheckpointBackend, SnapshotContext, SnapshotId,
    SnapshotScope, TraceProcess,
};

struct Backend(BtrfsBackend);

impl Backend {
    fn start() -> Self {
        Self::start_with_config(&env_path("ACTRAIL_BTRFS_TEST_CONFIG"))
    }

    fn start_with_config(config: &Path) -> Self {
        let config = fs::read_to_string(config).unwrap();
        Self(BtrfsBackend::new(&config).unwrap())
    }

    fn bind(&mut self, root: &Path, excludes: &[PathBuf]) -> Result<(), BackendError> {
        self.0
            .connect(&context(root, excludes, trace_process()))
            .into_result()
    }

    fn save(&mut self) -> SnapshotId {
        self.0.snapshot().unwrap()
    }

    fn restore(&mut self, id: &SnapshotId) -> Result<RVec<SnapshotId>, BackendError> {
        self.0
            .restore(id)
            .into_result()
            .map_err(|failure| failure.error)
    }

    fn delete(&mut self, id: &SnapshotId) {
        self.0.delete(id).unwrap();
    }
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
}

fn namespace(kind: &str) -> String {
    fs::read_link(format!("/proc/self/ns/{kind}"))
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn trace_process() -> TraceProcess {
    TraceProcess {
        host_pid: std::process::id(),
        start_time_ticks: 0,
        pid_namespace: namespace("pid").into(),
        mount_namespace: namespace("mnt").into(),
        cwd: std::env::current_dir()
            .unwrap()
            .display()
            .to_string()
            .into(),
    }
}

fn context(root: &Path, excludes: &[PathBuf], trace_process: TraceProcess) -> SnapshotContext {
    SnapshotContext {
        trace_id: "trace-1".into(),
        trace_process,
        scopes: vec![SnapshotScope {
            id: "project".into(),
            root: root.display().to_string().into(),
            excludes: excludes
                .iter()
                .map(|exclude| exclude.display().to_string().into())
                .collect(),
        }]
        .into(),
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// Owned by someone other than root, so a restore that copies a snapshot placeholder over it
/// shows up.
fn create_subvolume(path: &Path) {
    let created = Command::new("btrfs")
        .args(["subvolume", "create"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    chown(path, Some(1000), Some(1000)).unwrap();
}

/// The tests share one root, and connect refuses a root that still holds a nested subvolume.
fn delete_subvolume(path: &Path) {
    let deleted = Command::new("btrfs")
        .args(["subvolume", "delete"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn save_restore_delete_and_disconnect_round_trip() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    // A subvolume the Agent creates after connect is left alone by restore.
    let nested = root.join("cache");
    fs::write(root.join("a.txt"), "one").unwrap();
    fs::create_dir_all(root.join("dir")).unwrap();
    fs::write(root.join("dir/b.txt"), "keep").unwrap();

    let mut backend = Backend::start();
    backend.bind(&root, &[]).unwrap();
    let first = backend.save();

    create_subvolume(&nested);
    fs::write(nested.join("y"), "v1").unwrap();
    fs::write(root.join("a.txt"), "two").unwrap();
    fs::write(root.join("new.txt"), "new").unwrap();
    fs::remove_file(root.join("dir/b.txt")).unwrap();
    let second = backend.save();
    fs::write(nested.join("y"), "v2").unwrap();
    let owner = fs::metadata(&nested).unwrap().uid();

    backend.restore(&first).unwrap();
    assert_eq!(read(&root.join("a.txt")), "one");
    assert!(!root.join("new.txt").exists());
    assert_eq!(read(&root.join("dir/b.txt")), "keep");
    assert_eq!(
        read(&nested.join("y")),
        "v2",
        "nested subvolume must stay as it is"
    );
    assert_eq!(
        fs::metadata(&nested).unwrap().uid(),
        owner,
        "nested subvolume owner must not change"
    );

    backend.restore(&second).unwrap();
    assert_eq!(read(&root.join("a.txt")), "two");
    assert_eq!(read(&root.join("new.txt")), "new");
    assert!(!root.join("dir/b.txt").exists());

    backend.restore(&first).unwrap();
    assert_eq!(read(&root.join("a.txt")), "one");

    backend.delete(&first);
    let error = backend.restore(&first).unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::InvalidSnapshot);
    backend.delete(&first);

    // Disconnecting frees the snapshot that is left.
    backend.0.disconnect().unwrap();
    let saved = backend.0.snapshot().into_result();
    assert!(saved.unwrap_err().message.starts_with("not_connected"));
    delete_subvolume(&nested);
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn connect_rejects_scopes_it_cannot_honor() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    let plain_dir = root.join("plain-dir");
    fs::create_dir_all(&plain_dir).unwrap();
    let mut backend = Backend::start();

    // Excludes are not supported, whatever they name.
    let error = backend
        .bind(&root, std::slice::from_ref(&plain_dir))
        .unwrap_err();
    assert_eq!(
        error.kind,
        BackendErrorKind::InvalidConfiguration,
        "{}",
        error.message
    );

    // A snapshot would leave a nested subvolume out, so connect refuses the root.
    let nested = plain_dir.join("nested");
    create_subvolume(&nested);
    let error = backend.bind(&root, &[]).unwrap_err();
    assert!(
        error.message.starts_with("invalid_scope") && error.message.contains("nested"),
        "{}",
        error.message
    );
    delete_subvolume(&nested);

    let error = backend.bind(Path::new("/tmp"), &[]).unwrap_err();
    assert_eq!(
        error.kind,
        BackendErrorKind::InvalidConfiguration,
        "{}",
        error.message
    );

    let mut elsewhere = trace_process();
    elsewhere.mount_namespace = "mnt:[0]".into();
    let error = backend
        .0
        .connect(&context(&root, &[], elsewhere))
        .into_result()
        .unwrap_err();
    assert!(error.message.starts_with("unsupported_namespace"));

    fs::remove_dir(plain_dir).unwrap();
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn restore_preserves_directory_attributes_around_live_subvolumes() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    let nested = root.join("node_modules");
    let original = fs::metadata(&root).unwrap();
    let mut backend = Backend::start();
    backend.bind(&root, &[]).unwrap();
    // Created after connect, so restore walks the root entry by entry around it.
    create_subvolume(&nested);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
    xattr::set(&root, "user.checkpoint", b"saved").unwrap();
    let saved = fs::metadata(&root).unwrap();
    let nested_owner = fs::metadata(&nested).unwrap().uid();
    let object = backend.save();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    chown(&root, Some(1234), Some(1234)).unwrap();
    xattr::set(&root, "user.checkpoint", b"changed").unwrap();
    xattr::set(&root, "user.later", b"remove").unwrap();
    fs::write(nested.join("preserved"), "nested live data").unwrap();
    backend.restore(&object).unwrap();
    let restored = fs::metadata(&root).unwrap();
    assert_eq!(restored.mode() & 0o7777, saved.mode() & 0o7777);
    assert_eq!((restored.uid(), restored.gid()), (saved.uid(), saved.gid()));
    assert_eq!(restored.modified().unwrap(), saved.modified().unwrap());
    assert_eq!(
        xattr::get(&root, "user.checkpoint").unwrap(),
        Some(b"saved".to_vec())
    );
    assert_eq!(xattr::get(&root, "user.later").unwrap(), None);
    assert_eq!(read(&nested.join("preserved")), "nested live data");
    assert_eq!(fs::metadata(&nested).unwrap().uid(), nested_owner);
    xattr::remove(&root, "user.checkpoint").unwrap();
    fs::set_permissions(&root, original.permissions()).unwrap();
    delete_subvolume(&nested);
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn restore_keeps_hard_links_across_subtrees() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    fs::create_dir_all(root.join("links-a")).unwrap();
    fs::create_dir_all(root.join("links-b")).unwrap();
    let first = root.join("links-a/first");
    let second = root.join("links-b/second");
    let third = root.join("third");
    fs::write(&first, "saved").unwrap();
    fs::hard_link(&first, &second).unwrap();
    fs::hard_link(&first, &third).unwrap();
    let mut backend = Backend::start();
    backend.bind(&root, &[]).unwrap();
    let object = backend.save();
    fs::remove_file(&second).unwrap();
    fs::write(&second, "separate").unwrap();
    fs::remove_file(&third).unwrap();
    fs::write(&third, "separate").unwrap();
    backend.restore(&object).unwrap();
    assert_eq!(read(&first), "saved");
    assert_eq!(
        fs::metadata(&first).unwrap().ino(),
        fs::metadata(&second).unwrap().ino()
    );
    assert_eq!(
        fs::metadata(&first).unwrap().ino(),
        fs::metadata(&third).unwrap().ino()
    );
    fs::write(&second, "linked").unwrap();
    assert_eq!(read(&first), "linked");
    assert_eq!(read(&third), "linked");
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn restores_large_flat_directories_beside_live_subvolumes() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    for index in 0..24000 {
        fs::write(root.join(format!("flat-{index:05}")), "").unwrap();
    }
    let mut backend = Backend::start();
    backend.bind(&root, &[]).unwrap();
    // The subvolume makes restore copy the root's files in batches instead of as one tree.
    let nested = root.join("node_modules");
    create_subvolume(&nested);
    let object = backend.save();
    fs::write(root.join("flat-23999"), "changed").unwrap();
    backend.restore(&object).unwrap();
    assert_eq!(read(&root.join("flat-23999")), "");
    delete_subvolume(&nested);
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn operations_on_the_same_subvolume_serialize_across_instance_stores() {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    let original_config = env_path("ACTRAIL_BTRFS_TEST_CONFIG");
    let runtime = original_config.parent().unwrap();
    let started = runtime.join("copy-started");
    let release = runtime.join("copy-release");
    let wrapper = runtime.join("blocking-copy");
    fs::write(&wrapper, format!(
        "#!/bin/sh\ntouch '{}'\nwhile [ ! -e '{}' ]; do sleep .01; done\nexec /usr/bin/cp \"$@\"\n",
        started.display(), release.display(),
    )).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let mut backends = Vec::new();
    for name in ["a", "b"] {
        let mut config: serde_json::Value =
            serde_json::from_slice(&fs::read(&original_config).unwrap()).unwrap();
        config["copy_path"] = wrapper.display().to_string().into();
        let path = runtime.join(format!("config-{name}.json"));
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        backends.push(Backend::start_with_config(&path));
    }
    let mut backend_b = backends.pop().unwrap();
    let mut backend_a = backends.pop().unwrap();
    backend_a.bind(&root, &[]).unwrap();
    backend_b.bind(&root, &[]).unwrap();
    fs::write(root.join("serial-before"), "saved").unwrap();
    let object_a = backend_a.save();
    fs::remove_file(root.join("serial-before")).unwrap();
    fs::write(root.join("serial-after"), "later").unwrap();
    let restoring = thread::spawn(move || backend_a.restore(&object_a));
    for _ in 0..500 {
        if started.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists(), "restore reached the copy stage");
    let (sent, received) = mpsc::channel();
    let saving = thread::spawn(move || {
        let object = backend_b.save();
        sent.send(()).unwrap();
        (backend_b, object)
    });
    let before_release = received.recv_timeout(Duration::from_millis(100));
    fs::write(release, "").unwrap();
    assert!(
        matches!(before_release, Err(mpsc::RecvTimeoutError::Timeout)),
        "save must wait for restore"
    );
    restoring.join().unwrap().unwrap();
    let (mut backend_b, object_b) = saving.join().unwrap();
    fs::write(root.join("serial-before"), "dirty").unwrap();
    backend_b.restore(&object_b).unwrap();
    assert_eq!(read(&root.join("serial-before")), "saved");
    assert!(!root.join("serial-after").exists());
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn refuses_to_remove_ancestors_of_live_nested_subvolumes() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    let mut backend = Backend::start();
    backend.bind(&root, &[]).unwrap();
    for (name, saved_file) in [("new-ancestor", false), ("file-ancestor", true)] {
        let ancestor = root.join(name);
        if saved_file {
            fs::write(&ancestor, "saved file").unwrap();
        }
        let object = backend.save();
        if saved_file {
            fs::remove_file(&ancestor).unwrap();
        }
        fs::create_dir(&ancestor).unwrap();
        let nested = ancestor.join("keep");
        create_subvolume(&nested);
        fs::write(nested.join("valuable"), "keep this data").unwrap();

        let error = backend.restore(&object).unwrap_err();
        assert!(error.message.starts_with("nested_subvolume_conflict"));
        assert_eq!(read(&nested.join("valuable")), "keep this data");
        delete_subvolume(&nested);
        fs::remove_dir(&ancestor).unwrap();
    }
}

#[test]
#[ignore = "needs a real Btrfs filesystem; run integrations/checkpoint/filesystem/btrfs/tests/run-real.sh"]
fn restore_removes_file_attributes_absent_from_the_snapshot() {
    let root = env_path("ACTRAIL_BTRFS_TEST_ROOT");
    let file = root.join("file-attributes");
    fs::write(&file, "saved").unwrap();
    xattr::set(&file, "user.saved", b"old").unwrap();
    let mut backend = Backend::start();
    backend.bind(&root, &[]).unwrap();
    let object = backend.save();
    fs::write(&file, "changed").unwrap();
    xattr::set(&file, "user.saved", b"changed").unwrap();
    xattr::set(&file, "user.added", b"new").unwrap();
    backend.restore(&object).unwrap();
    assert_eq!(read(&file), "saved");
    assert_eq!(
        xattr::get(&file, "user.saved").unwrap(),
        Some(b"old".to_vec())
    );
    assert_eq!(xattr::get(&file, "user.added").unwrap(), None);
}

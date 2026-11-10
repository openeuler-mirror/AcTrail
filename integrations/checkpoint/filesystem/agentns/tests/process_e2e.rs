//! Drives the backend against a stand-in librns (fake_rns.c).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use abi_stable::library::lib_header_from_path;
use abi_stable::std_types::RVec;
use actrail_ckpt_fs_agentns::AgentNsBackend;
use checkpoint::{
    BackendConfig, BackendError, BackendErrorKind, CheckpointModuleRef,
    RestorableCheckpointBackend, SnapshotContext, SnapshotId, SnapshotScope, TraceProcess,
};

struct Backend(AgentNsBackend);

impl Backend {
    fn start(config: &str) -> Self {
        Self(AgentNsBackend::new(config).unwrap())
    }

    fn connect(&mut self, scopes: Vec<SnapshotScope>) -> Result<(), BackendError> {
        self.0.connect(&context(scopes)).into_result()
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
}

struct Setup {
    root: PathBuf,
    config: String,
}

impl Setup {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("actrail-agentns-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let library = root.join("librns.so");
        let compiled = Command::new("cc")
            .args(["-shared", "-fPIC", "-O2", "-o"])
            .arg(&library)
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_rns.c"))
            .status()
            .unwrap();
        assert!(compiled.success());
        let config = serde_json::json!({
            "area_root": root.join("areas"),
            "library_path": library,
            "extra_excluded_paths": ["/var/lib/actrail"],
        })
        .to_string();
        Self { root, config }
    }

    /// The one area directory of the connection under test.
    fn area(&self) -> PathBuf {
        fs::read_dir(self.root.join("areas"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_none())
            .expect("the connection has an area")
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.area().join("calls"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn this_process() -> TraceProcess {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    let after_name = &stat[stat.rfind(')').unwrap() + 1..];
    TraceProcess {
        host_pid: std::process::id(),
        start_time_ticks: after_name
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap(),
        pid_namespace: fs::read_link("/proc/self/ns/pid")
            .unwrap()
            .to_string_lossy()
            .into_owned()
            .into(),
        mount_namespace: fs::read_link("/proc/self/ns/mnt")
            .unwrap()
            .to_string_lossy()
            .into_owned()
            .into(),
        cwd: "/".into(),
    }
}

fn context(scopes: Vec<SnapshotScope>) -> SnapshotContext {
    SnapshotContext {
        trace_id: "trace-7".into(),
        trace_process: this_process(),
        scopes: scopes.into(),
    }
}

fn host_scope(root: &str) -> SnapshotScope {
    SnapshotScope {
        id: "host".into(),
        root: root.into(),
        excludes: vec!["/work/project".into()].into(),
    }
}

#[test]
fn saves_restore_with_a_sealed_tail() {
    let setup = Setup::new("lifecycle");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let calls = setup.calls();
    let pid = std::process::id();
    assert!(
        calls[0].starts_with(&format!("create actrail-trace-7 {pid} ")),
        "{calls:?}"
    );
    for exclude in ["/var/lib/actrail", "/work/project"] {
        assert!(calls[0].contains(&format!("{exclude},")), "{calls:?}");
    }
    assert!(calls[0].contains(&format!("{},", setup.root.join("areas").display())));
    assert_eq!(calls[1], format!("attach actrail-trace-7 {pid} 1 0"));

    let first = backend.save();
    let second = backend.save();
    assert_eq!((&first.0[..], &second.0[..]), (&b"1"[..], &b"2"[..]));

    let invalidated = backend.restore(&first).unwrap();
    assert_eq!(invalidated.as_slice(), std::slice::from_ref(&second));
    let calls = setup.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        [
            "ckpt actrail-trace-7 actrail restore",
            "restore actrail-trace-7 1"
        ],
        "the tail is sealed before the rollback"
    );
    assert_eq!(
        backend.restore(&second).unwrap_err().kind,
        BackendErrorKind::InvalidSnapshot
    );
}

#[test]
fn only_disconnect_frees_the_area() {
    let setup = Setup::new("disconnect");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let object = backend.save();
    backend.0.delete(&object).unwrap();
    backend.0.delete(&object).unwrap();
    let area = setup.area();
    let freed = area.with_extension("del");
    assert!(!freed.exists(), "deleting checkpoints keeps the area");

    backend.0.disconnect().unwrap();
    assert!(freed.exists());
    assert!(!area.exists());
    let saved = backend.0.snapshot().into_result();
    assert!(saved.unwrap_err().message.starts_with("not_connected"));
}

#[test]
fn a_failed_restore_reports_what_it_invalidated() {
    let setup = Setup::new("failed-restore");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    let second = backend.save();
    fs::write(setup.area().join("fail-restore"), "").unwrap();
    let failure = backend.0.restore(&first).into_result().unwrap_err();
    assert_eq!(
        failure.invalidated.as_slice(),
        std::slice::from_ref(&second)
    );
}

#[test]
fn a_rejected_start_preserves_checkpoints_and_allows_another_restore() {
    for reason in ["busy", "missing"] {
        let setup = Setup::new(&format!("rejected-start-{reason}"));
        let mut backend = Backend::start(&setup.config);
        backend.connect(vec![host_scope("/")]).unwrap();
        let first = backend.save();
        let second = backend.save();
        let rejection = setup.area().join("reject-restore");
        fs::write(&rejection, reason).unwrap();

        let failure = backend.0.restore(&first).into_result().unwrap_err();
        assert!(failure.invalidated.is_empty(), "START was not registered");
        fs::remove_file(rejection).unwrap();
        assert!(backend.restore(&second).unwrap().is_empty());
        assert_eq!(
            backend.restore(&first).unwrap().as_slice(),
            std::slice::from_ref(&second)
        );
        assert!(
            setup.calls().iter().all(|call| {
                !call.starts_with("continue ") && !call.starts_with("unknown-request ")
            }),
            "a rejected START must not leave a request to continue"
        );
    }
}

#[test]
fn an_accepted_failed_restore_continues_the_original_request() {
    let setup = Setup::new("continue-accepted");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    let failure_marker = setup.area().join("fail-restore");
    fs::write(&failure_marker, "").unwrap();
    backend.restore(&first).unwrap_err();

    fs::remove_file(failure_marker).unwrap();
    backend.restore(&first).unwrap();
    assert!(
        setup
            .calls()
            .iter()
            .any(|call| call == "continue actrail-trace-7")
    );
    backend.save();
}

#[test]
fn a_lost_result_is_resolved_before_checking_the_next_target() {
    let setup = Setup::new("lost-restore-result");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    let second = backend.save();
    let lost_reply = setup.area().join("lose-restore-result");
    fs::write(&lost_reply, "").unwrap();
    let failure = backend.0.restore(&first).into_result().unwrap_err();
    assert!(failure.invalidated.is_empty(), "the result is not known yet");

    fs::remove_file(lost_reply).unwrap();
    let failure = backend.0.restore(&second).into_result().unwrap_err();
    assert_eq!(failure.error.kind, BackendErrorKind::InvalidSnapshot);
    assert_eq!(failure.invalidated.as_slice(), std::slice::from_ref(&second));
    let calls = setup.calls();
    assert_eq!(calls.last().unwrap(), "query actrail-trace-7");
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.starts_with("restore "))
            .count(),
        1,
        "confirm the original request without executing another rollback"
    );
    backend.restore(&first).unwrap();
}

#[test]
fn resolving_a_lost_result_preserves_checkpoints_saved_after_it() {
    let setup = Setup::new("save-after-lost-result");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    let second = backend.save();
    let lost_reply = setup.area().join("lose-restore-result");
    fs::write(&lost_reply, "").unwrap();
    backend.restore(&first).unwrap_err();
    let after_restore = backend.save();

    fs::remove_file(lost_reply).unwrap();
    let invalidated = backend.restore(&after_restore).unwrap();
    assert_eq!(invalidated.as_slice(), std::slice::from_ref(&second));
    assert!(!invalidated.contains(&after_restore));
    backend.restore(&after_restore).unwrap();
}

#[test]
fn an_uncertain_unregistered_start_does_not_override_the_next_target() {
    let setup = Setup::new("uncertain-rejection");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    let second = backend.save();
    let rejection = setup.area().join("reject-restore");
    fs::write(&rejection, "fault").unwrap();
    backend.restore(&first).unwrap_err();

    fs::remove_file(rejection).unwrap();
    assert!(backend.restore(&second).unwrap().is_empty());
    assert_eq!(
        backend.restore(&first).unwrap().as_slice(),
        std::slice::from_ref(&second)
    );
}

#[test]
fn a_lost_unfinished_result_is_queried_then_continued() {
    let setup = Setup::new("lost-unfinished-result");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    let second = backend.save();
    let area = setup.area();
    fs::write(area.join("fail-restore"), "").unwrap();
    fs::write(area.join("lose-restore-result"), "").unwrap();
    backend.restore(&first).unwrap_err();

    fs::remove_file(area.join("fail-restore")).unwrap();
    fs::remove_file(area.join("lose-restore-result")).unwrap();
    assert_eq!(
        backend.restore(&first).unwrap().as_slice(),
        std::slice::from_ref(&second)
    );
    let calls = setup.calls();
    let query = calls
        .iter()
        .position(|call| call == "query actrail-trace-7")
        .unwrap();
    assert_eq!(calls[query + 1], "continue actrail-trace-7");
}

#[test]
fn disconnect_drops_a_restore_that_stopped_short() {
    let setup = Setup::new("disconnect-unfinished");
    let mut backend = Backend::start(&setup.config);
    backend.connect(vec![host_scope("/")]).unwrap();
    let first = backend.save();
    fs::write(setup.area().join("fail-restore"), "").unwrap();
    backend.restore(&first).unwrap_err();
    let area = setup.area();
    // Continuing the restore would fail, so this only succeeds without it.
    backend.0.disconnect().unwrap();
    assert!(!area.exists());
}

#[test]
fn the_library_exports_the_checkpoint_module() {
    let setup = Setup::new("module");
    let library = std::env::current_exe()
        .unwrap()
        .with_file_name("libactrail_ckpt_fs_agentns.so");
    let module = lib_header_from_path(&library)
        .unwrap()
        .init_root_module::<CheckpointModuleRef>()
        .unwrap();
    let mut backend = module.create()(BackendConfig {
        json: setup.config.as_str().into(),
    })
    .unwrap();
    backend.connect(&context(vec![host_scope("/")])).unwrap();
    backend.snapshot().unwrap();
    backend.disconnect().unwrap();
}

#[test]
fn connecting_needs_one_root_scope() {
    let setup = Setup::new("connect");
    let mut backend = Backend::start(&setup.config);
    for scopes in [
        vec![host_scope("/work")],
        vec![host_scope("/"), host_scope("/")],
    ] {
        assert_eq!(
            backend.connect(scopes).unwrap_err().kind,
            BackendErrorKind::InvalidConfiguration
        );
    }
}

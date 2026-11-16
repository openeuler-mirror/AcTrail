// Tests for the corresponding production module.
use std::os::unix::net::UnixListener;

use super::{ensure_runtime_socket_available, write_pid_file};

#[test]
fn write_pid_file_reclaims_stale_owner() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("actraild.pid");
    let mut child = std::process::Command::new("/bin/true")
        .spawn()
        .expect("spawn short-lived process");
    let stale_pid = child.id();
    child.wait().expect("wait for short-lived process");
    std::fs::write(&path, format!("{stale_pid}\n")).expect("write stale pid file");

    write_pid_file(&path, std::process::id()).expect("replace stale pid file");

    assert_eq!(
        std::fs::read_to_string(path).expect("read replacement pid file"),
        format!("{}\n", std::process::id())
    );
}

#[test]
fn write_pid_file_preserves_live_owner() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("actraild.pid");
    std::fs::write(&path, format!("{}\n", std::process::id())).expect("write live pid file");

    let error = write_pid_file(&path, std::process::id()).expect_err("reject live owner");

    assert!(error.contains("belongs to running pid"));
    assert_eq!(
        std::fs::read_to_string(path).expect("read preserved pid file"),
        format!("{}\n", std::process::id())
    );
}

#[test]
fn runtime_socket_check_removes_stale_socket() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("control.sock");
    drop(UnixListener::bind(&path).expect("bind temporary listener"));

    let mut result = ensure_runtime_socket_available("control", &path);
    for _ in 0..20 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
        result = ensure_runtime_socket_available("control", &path);
    }
    result.expect("remove stale socket");

    assert!(!path.exists());
}

#[test]
fn runtime_socket_check_preserves_active_listener() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("control.sock");
    let _listener = UnixListener::bind(&path).expect("bind temporary listener");

    let error =
        ensure_runtime_socket_available("control", &path).expect_err("reject active listener");

    assert!(error.contains("already has an active listener"));
    assert!(path.exists());
}

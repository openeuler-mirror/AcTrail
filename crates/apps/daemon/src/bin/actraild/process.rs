//! Local process supervision for `actraild start/stop/status`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use config_core::daemon::OperatorConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonProcessState {
    Running { pid: u32 },
    Stopped,
    StalePid { pid: u32 },
    StaleSocket,
}

pub fn start_daemon(config_path: &Path, config: &OperatorConfig) -> Result<(), String> {
    ensure_start_preconditions(config)?;
    create_parent_directory(&config.log_path, "log directory")?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.log_path)
        .map_err(|error| format!("open log {}: {error}", config.log_path.display()))?;
    let stderr = log
        .try_clone()
        .map_err(|error| format!("clone log {}: {error}", config.log_path.display()))?;
    let mut command = Command::new(
        std::env::current_exe().map_err(|error| format!("resolve current executable: {error}"))?,
    );
    command
        .arg("run")
        .arg("--config")
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    unsafe {
        command.pre_exec(|| {
            let session_id = libc::setsid();
            if session_id < libc::pid_t::default() {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn actraild: {error}"))?;
    wait_until_started(&mut child, config)
}

pub fn stop_daemon(config: &OperatorConfig) -> Result<(), String> {
    match status_daemon(config)? {
        DaemonProcessState::Stopped => {
            println!("actraild already stopped");
            Ok(())
        }
        DaemonProcessState::StaleSocket => Err(format!(
            "socket path {} exists without pid file; remove it explicitly after confirming no daemon owns it",
            config.socket_path.display()
        )),
        DaemonProcessState::StalePid { pid } => Err(format!(
            "stale pid file {} points to non-running pid {}; remove it explicitly",
            config.pid_file.display(),
            pid
        )),
        DaemonProcessState::Running { pid } => {
            if signal_process(pid, libc::SIGTERM)? {
                wait_until_stopped(pid, config)?;
            }
            remove_runtime_file(&config.pid_file)?;
            remove_runtime_file(&config.socket_path)?;
            println!("actraild stopped pid={pid}");
            Ok(())
        }
    }
}

pub fn status_daemon(config: &OperatorConfig) -> Result<DaemonProcessState, String> {
    let Some(pid) = read_pid_file(&config.pid_file)? else {
        if config.socket_path.exists() {
            return Ok(DaemonProcessState::StaleSocket);
        }
        return Ok(DaemonProcessState::Stopped);
    };
    if process_exists(pid)? {
        Ok(DaemonProcessState::Running { pid })
    } else {
        Ok(DaemonProcessState::StalePid { pid })
    }
}

pub fn write_pid_file(path: &Path, pid: u32) -> Result<(), String> {
    if pid == u32::default() {
        return Err("current process id must not be zero".to_string());
    }
    create_parent_directory(path, "pid directory")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create pid file {}: {error}", path.display()))?;
    writeln!(file, "{pid}").map_err(|error| format!("write pid file {}: {error}", path.display()))
}

pub fn remove_runtime_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove {}: {error}", path.display())),
    }
}

fn create_parent_directory(path: &Path, label: &str) -> Result<(), String> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent)
        .map_err(|error| format!("create {label} {}: {error}", parent.display()))
}

fn ensure_start_preconditions(config: &OperatorConfig) -> Result<(), String> {
    match status_daemon(config)? {
        DaemonProcessState::Running { pid } => {
            return Err(format!("actraild already running pid={pid}"));
        }
        DaemonProcessState::StalePid { pid } => {
            return Err(format!(
                "stale pid file {} points to non-running pid {}; remove it explicitly",
                config.pid_file.display(),
                pid
            ));
        }
        DaemonProcessState::StaleSocket => {
            return Err(format!(
                "socket path {} exists without pid file; remove it explicitly after confirming no daemon owns it",
                config.socket_path.display()
            ));
        }
        DaemonProcessState::Stopped => {}
    }
    if config.socket_path.exists() {
        return Err(format!(
            "socket path already exists: {}; remove it explicitly after confirming no daemon owns it",
            config.socket_path.display()
        ));
    }
    if config.payload_config.tls.enabled && config.payload_config.tls.capture_backend.is_sync() {
        ensure_auxiliary_socket_available(
            "TLS sync event",
            &config.payload_config.tls.sync_event_socket_path,
        )?;
    }
    Ok(())
}

fn ensure_auxiliary_socket_available(label: &str, path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "inspect {label} socket {}: {error}",
                path.display()
            ));
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(format!(
            "{label} socket path {} exists but is not a Unix socket; remove it explicitly",
            path.display()
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(format!(
            "{label} socket {} already has an active listener",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => remove_runtime_file(path),
        Err(error) => Err(format!(
            "{label} socket {} exists but could not be verified stale: {error}; remove it explicitly",
            path.display()
        )),
    }
}

fn wait_until_started(
    child: &mut std::process::Child,
    config: &OperatorConfig,
) -> Result<(), String> {
    let wait_for = Duration::from_millis(config.startup_wait_ms);
    let poll_every = Duration::from_millis(config.supervision_poll_interval_ms);
    let started_at = Instant::now();
    while started_at.elapsed() < wait_for {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("check actraild child status: {error}"))?
        {
            return Err(format!(
                "actraild exited before ready with status {status}; log={}",
                config.log_path.display()
            ));
        }
        if config.socket_path.exists() && config.pid_file.exists() {
            println!(
                "actraild started pid={} socket={}",
                child.id(),
                config.socket_path.display()
            );
            return Ok(());
        }
        std::thread::sleep(poll_every);
    }
    Err(format!(
        "actraild did not become ready within {} ms; log={}",
        config.startup_wait_ms,
        config.log_path.display()
    ))
}

fn wait_until_stopped(pid: u32, config: &OperatorConfig) -> Result<(), String> {
    let wait_for = Duration::from_millis(config.shutdown_wait_ms);
    let poll_every = Duration::from_millis(config.supervision_poll_interval_ms);
    let started_at = Instant::now();
    while started_at.elapsed() < wait_for {
        if !process_exists(pid)? {
            return Ok(());
        }
        std::thread::sleep(poll_every);
    }
    Err(format!(
        "actraild pid={} did not stop within {} ms",
        pid, config.shutdown_wait_ms
    ))
}

fn read_pid_file(path: &Path) -> Result<Option<u32>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read pid file {}: {error}", path.display())),
    };
    let pid = raw
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid pid file {}: {error}", path.display()))?;
    if pid == u32::default() {
        return Err(format!(
            "invalid pid file {}: pid must not be zero",
            path.display()
        ));
    }
    Ok(Some(pid))
}

fn process_exists(pid: u32) -> Result<bool, String> {
    signal_process(pid, process_exists_signal())
}

fn process_exists_signal() -> libc::c_int {
    libc::c_int::default()
}

fn signal_process(pid: u32, signal: libc::c_int) -> Result<bool, String> {
    let raw_pid =
        libc::pid_t::try_from(pid).map_err(|error| format!("invalid pid {pid}: {error}"))?;
    let result = unsafe { libc::kill(raw_pid, signal) };
    if result == libc::c_int::default() {
        return Ok(true);
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(errno) if errno == libc::ESRCH => Ok(false),
        Some(errno) if errno == libc::EPERM => Ok(true),
        Some(errno) => Err(format!("signal pid={pid} failed with errno {errno}")),
        None => Err(format!("signal pid={pid} failed")),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn auxiliary_socket_precondition_removes_stale_socket_file() {
        let path = temp_socket_path("stale");
        let _listener = UnixListener::bind(&path).unwrap();
        drop(_listener);

        ensure_auxiliary_socket_available("test", &path).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn auxiliary_socket_precondition_rejects_active_listener() {
        let path = temp_socket_path("active");
        let listener = UnixListener::bind(&path).unwrap();

        let error = ensure_auxiliary_socket_available("test", &path).unwrap_err();

        assert!(error.contains("already has an active listener"));
        drop(listener);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn auxiliary_socket_precondition_rejects_non_socket_path() {
        let path = temp_socket_path("file");
        fs::write(&path, b"not a socket").unwrap();

        let error = ensure_auxiliary_socket_available("test", &path).unwrap_err();

        assert!(error.contains("is not a Unix socket"));
        fs::remove_file(path).unwrap();
    }

    fn temp_socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "actrail-aux-socket-{name}-{}.sock",
            std::process::id()
        ))
    }
}

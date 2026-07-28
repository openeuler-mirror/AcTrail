//! Long-running cluster upload scheduling.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use config_core::daemon::{ClusterReportConfig, OperatorConfig};

use super::{UploadSummary, upload_once};

const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(200);

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub(super) fn run(config_path: &Path) -> Result<(), String> {
    SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    install_shutdown_handlers()?;

    let initial_config = OperatorConfig::load(config_path)?;
    let mut upload_lock =
        UploadLoopLock::acquire(&initial_config.cluster.report.state_path, config_path)?;
    let mut last_report = initial_config.cluster.report;
    let mut next_retry_secs = None;
    let mut last_enabled = None;

    println!(
        "upload-loop started: config={} lock={}",
        config_path.display(),
        upload_lock.path.display()
    );

    loop {
        if shutdown_requested() {
            println!("upload-loop stopped");
            return Ok(());
        }

        let config = match OperatorConfig::load(config_path) {
            Ok(config) => config,
            Err(error) => {
                let delay_secs = retry_delay_secs(&last_report, &mut next_retry_secs);
                eprintln!("upload-loop config reload failed: {error}; retrying in {delay_secs}s");
                if wait_for_shutdown(Duration::from_secs(delay_secs)) {
                    println!("upload-loop stopped");
                    return Ok(());
                }
                continue;
            }
        };
        upload_lock.ensure_for(&config.cluster.report.state_path, config_path)?;
        last_report = config.cluster.report.clone();

        let enabled = config.cluster.enabled && config.cluster.report.enabled;
        if last_enabled != Some(enabled) {
            if enabled {
                println!("upload-loop reporting enabled");
            } else {
                println!(
                    "upload-loop reporting disabled; waiting {}s before reloading config",
                    config.cluster.report.interval_secs
                );
            }
            last_enabled = Some(enabled);
        }

        if !enabled {
            next_retry_secs = None;
            if wait_for_shutdown(Duration::from_secs(config.cluster.report.interval_secs)) {
                println!("upload-loop stopped");
                return Ok(());
            }
            continue;
        }

        match upload_once(&config) {
            Ok(summary) if summary.failed == 0 => {
                log_successful_round(summary, config.cluster.report.interval_secs);
                next_retry_secs = None;
                if wait_for_shutdown(Duration::from_secs(config.cluster.report.interval_secs)) {
                    println!("upload-loop stopped");
                    return Ok(());
                }
            }
            Ok(summary) => {
                let delay_secs = retry_delay_secs(&config.cluster.report, &mut next_retry_secs);
                eprintln!(
                    "upload-loop round had {} failed trace(s); retrying in {delay_secs}s",
                    summary.failed
                );
                if wait_for_shutdown(Duration::from_secs(delay_secs)) {
                    println!("upload-loop stopped");
                    return Ok(());
                }
            }
            Err(error) => {
                let delay_secs = retry_delay_secs(&config.cluster.report, &mut next_retry_secs);
                eprintln!("upload-loop round failed: {error}; retrying in {delay_secs}s");
                if wait_for_shutdown(Duration::from_secs(delay_secs)) {
                    println!("upload-loop stopped");
                    return Ok(());
                }
            }
        }
    }
}

fn log_successful_round(summary: UploadSummary, interval_secs: u64) {
    println!(
        "upload-loop round successful: uploaded={} skipped_unchanged={}; next run in {}s",
        summary.uploaded, summary.skipped_unchanged, interval_secs
    );
}

fn retry_delay_secs(report: &ClusterReportConfig, next_retry_secs: &mut Option<u64>) -> u64 {
    let maximum = report.max_retry_backoff_secs;
    let initial = report.retry_backoff_secs.min(maximum);
    let delay = next_retry_secs.unwrap_or(initial).min(maximum);
    *next_retry_secs = Some(delay.saturating_mul(2).min(maximum));
    delay
}

fn wait_for_shutdown(duration: Duration) -> bool {
    let started = Instant::now();
    loop {
        if shutdown_requested() {
            return true;
        }
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return false;
        }
        thread::sleep(remaining.min(SHUTDOWN_POLL_INTERVAL));
    }
}

fn install_shutdown_handlers() -> Result<(), String> {
    install_shutdown_handler(libc::SIGTERM)?;
    install_shutdown_handler(libc::SIGINT)
}

fn install_shutdown_handler(signal: libc::c_int) -> Result<(), String> {
    let previous = unsafe {
        libc::signal(
            signal,
            handle_shutdown_signal as *const () as libc::sighandler_t,
        )
    };
    if previous == libc::SIG_ERR {
        return Err(format!(
            "install upload-loop signal handler {signal}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

struct UploadLoopLock {
    path: PathBuf,
    _file: File,
}

impl UploadLoopLock {
    fn acquire(state_path: &Path, config_path: &Path) -> Result<Self, String> {
        let lock_path = Self::path_for(state_path);
        if let Some(parent) = lock_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "create upload-loop lock directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&lock_path)
            .map_err(|error| format!("open upload-loop lock {}: {error}", lock_path.display()))?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error
                .raw_os_error()
                .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
            {
                let owner = Self::read_owner(&mut file, &lock_path)?;
                let suffix = if owner.is_empty() {
                    String::new()
                } else {
                    format!(" ({owner})")
                };
                return Err(format!(
                    "another upload-loop owns {}{suffix}",
                    lock_path.display()
                ));
            }
            return Err(format!(
                "lock upload-loop path {}: {error}",
                lock_path.display()
            ));
        }
        file.set_len(0).map_err(|error| {
            format!("truncate upload-loop lock {}: {error}", lock_path.display())
        })?;
        write!(
            file,
            "pid={} config={}",
            std::process::id(),
            config_path.display()
        )
        .map_err(|error| format!("write upload-loop lock {}: {error}", lock_path.display()))?;
        file.sync_data()
            .map_err(|error| format!("sync upload-loop lock {}: {error}", lock_path.display()))?;
        Ok(Self {
            path: lock_path,
            _file: file,
        })
    }

    fn ensure_for(&mut self, state_path: &Path, config_path: &Path) -> Result<(), String> {
        let desired_path = Self::path_for(state_path);
        if desired_path == self.path {
            return Ok(());
        }
        let replacement = Self::acquire(state_path, config_path)?;
        *self = replacement;
        Ok(())
    }

    fn read_owner(file: &mut File, lock_path: &Path) -> Result<String, String> {
        file.seek(SeekFrom::Start(0))
            .map_err(|error| format!("seek upload-loop lock {}: {error}", lock_path.display()))?;
        let mut owner = String::new();
        file.read_to_string(&mut owner)
            .map_err(|error| format!("read upload-loop lock {}: {error}", lock_path.display()))?;
        Ok(owner.trim().to_string())
    }

    fn path_for(state_path: &Path) -> PathBuf {
        let mut raw: OsString = state_path.as_os_str().to_owned();
        raw.push(".upload-loop.lock");
        PathBuf::from(raw)
    }
}

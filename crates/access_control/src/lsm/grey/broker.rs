//! Bounded workers forward module requests to control plugins and return verdicts.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;
use std::thread::JoinHandle;

use super::LsmGreyHandler;
use super::protocol::{LsmGreyReply, LsmGreyRequest, REQUEST_BYTES};
use config_core::daemon::LsmGreyConfig;
use plugin_system::{ControlDecisionBudget, ControlVerdict};

use super::context::GreyContext;

pub(in crate::lsm) struct LsmGreyBroker {
    stop: Arc<OwnedFd>,
    workers: Vec<JoinHandle<()>>,
}

impl LsmGreyBroker {
    pub(in crate::lsm) fn start(
        config: &LsmGreyConfig,
        handler: Arc<dyn LsmGreyHandler>,
    ) -> Result<Self, String> {
        let device = Arc::new(Self::open_device(config)?);
        Self::start_workers(config, handler, device)
    }

    // Called only after the effective routing configuration selects LSM.
    fn open_device(config: &LsmGreyConfig) -> Result<File, String> {
        if unsafe { libc::geteuid() } != 0 {
            return Err("LSM grey requires a root daemon".into());
        }
        if !std::path::Path::new("/sys/module/actrail_lsm_grey").exists() {
            let output = std::process::Command::new("modprobe")
                .arg("actrail_lsm_grey")
                .output()
                .map_err(|error| format!("load LSM grey module: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "load LSM grey module: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&config.device_path)
            .map_err(|error| format!("open {}: {error}", config.device_path))
    }

    fn start_workers(
        config: &LsmGreyConfig,
        handler: Arc<dyn LsmGreyHandler>,
        device: Arc<File>,
    ) -> Result<Self, String> {
        let stop = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if stop < 0 {
            return Err(format!(
                "create grey stop event: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut broker = Self {
            stop: Arc::new(unsafe { OwnedFd::from_raw_fd(stop) }),
            workers: Vec::new(),
        };
        for index in 0..config.workers {
            let worker = GreyWorker {
                device: device.clone(),
                stop: broker.stop.clone(),
                handler: handler.clone(),
            };
            broker.workers.push(
                std::thread::Builder::new()
                    .name(format!("lsm-grey-{index}"))
                    .spawn(move || worker.run())
                    .map_err(|error| format!("spawn grey worker: {error}"))?,
            );
        }
        Ok(broker)
    }
}

impl Drop for LsmGreyBroker {
    fn drop(&mut self) {
        // Keep the event readable so every worker wakes; in-flight plugins retain
        // their request deadline. The last worker closes the exclusive session.
        let signal = 1_u64.to_ne_bytes();
        unsafe { libc::write(self.stop.as_raw_fd(), signal.as_ptr().cast(), signal.len()) };
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                tracing::warn!("LSM grey worker panicked during shutdown");
            }
        }
    }
}

struct GreyWorker {
    device: Arc<File>,
    stop: Arc<OwnedFd>,
    handler: Arc<dyn LsmGreyHandler>,
}

impl GreyWorker {
    fn run(self) {
        let mut descriptors = [
            libc::pollfd {
                fd: self.device.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.stop.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        loop {
            let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, -1) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                tracing::warn!(%error, "LSM grey poll failed");
                break;
            }
            if descriptors[1].revents != 0 {
                break;
            }
            if descriptors[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                tracing::warn!("LSM grey module session closed");
                break;
            }
            let mut bytes = [0_u8; REQUEST_BYTES];
            match self.device.as_ref().read(&mut bytes) {
                Ok(REQUEST_BYTES) => {}
                Ok(length) => {
                    tracing::warn!(length, "LSM grey device returned an incomplete request");
                    break;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => {
                    tracing::warn!(%error, "LSM grey request read failed");
                    break;
                }
            }
            let request = match LsmGreyRequest::decode(&bytes) {
                Ok(request) => request,
                Err(error) => {
                    tracing::warn!(%error, "LSM grey request decode failed");
                    continue;
                }
            };
            let reply = LsmGreyReply {
                request_id: request.request_id,
                decision: self.decide(&request),
            };
            let bytes = reply.encode();
            loop {
                match self.device.as_ref().write(&bytes) {
                    Ok(length) if length == bytes.len() => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        tracing::debug!(request_id = request.request_id, %error, "LSM grey reply was not accepted");
                        break;
                    }
                    Ok(length) => {
                        tracing::warn!(length, "LSM grey device accepted an incomplete reply");
                        break;
                    }
                }
            }
        }
    }

    fn decide(&self, request: &LsmGreyRequest) -> u32 {
        // The kernel owns pending requests and applies fallback at the deadline.
        // No userspace cache or policy lookup can replace this request snapshot.
        let fallback = u32::from(request.fallback_deny);
        let mut now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) } != 0 {
            return fallback;
        }
        let now_ns = (now.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(now.tv_nsec as u64);
        let remaining_ms = request.deadline_boottime_ns.saturating_sub(now_ns) / 1_000_000;
        if remaining_ms == 0 {
            return fallback;
        }
        let context = match GreyContext::project(request) {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(%error, "LSM grey context unavailable");
                return fallback;
            }
        };
        let result = self.handler.decide(
            &request.target,
            context,
            ControlDecisionBudget {
                timeout_ms: Some(remaining_ms),
            },
        );
        match result {
            Ok(verdict) => u32::from(verdict == ControlVerdict::Deny),
            Err(error) => {
                tracing::debug!(request_id = request.request_id, %error, "LSM grey plugin failed");
                fallback
            }
        }
    }
}

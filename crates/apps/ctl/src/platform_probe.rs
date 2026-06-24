//! Local platform probes and launch recommendation for actrailctl.

use config_core::daemon::{OperatorConfig, PayloadTlsConfig, PayloadTlsSeccompSyscall, PayloadTlsSyncRuntimeLibraryPath};
use control_contract::reply::DoctorReply;
use linux_platform::capability_probe::{CapabilityStatus, probe_no_new_privs, probe_unix_socket};
use tls_payload_sync::RuntimeLibraryPath;

use crate::launch::controlled::ControlledChild;
use crate::launch::seccomp::SeccompSetup;
use crate::output::format_reply;
use crate::transport::ControlClientPort;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchPlatformReport {
    pub control_socket: CapabilityStatus,
    pub tls_sync_socket: CapabilityStatus,
    pub no_new_privs: CapabilityStatus,
    pub seccomp_launch: CapabilityStatus,
    pub tls_sync_runtime_library: CapabilityStatus,
    pub daemon: Option<DoctorReply>,
}

pub fn run_platform_probe(config: &OperatorConfig) -> LaunchPlatformReport {
    let control_socket = probe_unix_socket(&config.socket_path);
    let tls_sync_socket = if config.payload_config.tls.enabled
        && config.payload_config.tls.capture_backend.is_sync()
    {
        probe_unix_socket(&config.payload_config.tls.sync_event_socket_path)
    } else {
        CapabilityStatus::ok("tls_sync_socket", "disabled by operator config")
    };
    let no_new_privs = probe_no_new_privs();
    let seccomp_launch = probe_seccomp_launch_capability(config.seccomp_notify.reserved_listener_fd);
    let tls_sync_runtime_library = probe_tls_sync_runtime_library(&config.payload_config.tls);
    LaunchPlatformReport {
        control_socket,
        tls_sync_socket,
        no_new_privs,
        seccomp_launch,
        tls_sync_runtime_library,
        daemon: None,
    }
}

pub fn probe_seccomp_launch_capability(reserved_listener_fd: u32) -> CapabilityStatus {
    let setup = match SeccompSetup::new(
        vec![PayloadTlsSeccompSyscall::Write],
        Vec::new(),
        4095,
        Vec::new(),
        reserved_listener_fd,
    ) {
        Ok(setup) => setup,
        Err(error) => return CapabilityStatus::unavailable("seccomp_launch", error),
    };
    match ControlledChild::probe_seccomp_launch_path(&setup) {
        Ok(()) => CapabilityStatus::ok(
            "seccomp_launch",
            "seccomp user notify and pidfd_getfd launch path available",
        ),
        Err(error) => CapabilityStatus::unavailable("seccomp_launch", error),
    }
}

pub fn attach_daemon_status(
    report: &mut LaunchPlatformReport,
    client: &mut impl ControlClientPort,
) {
    match client.send(control_contract::command::ControlCommand::Doctor(
        control_contract::command::DoctorCommand {
            request_id: model_core::ids::RequestId::new(1),
        },
    )) {
        Ok(control_contract::reply::ControlReply::Doctor(reply)) => {
            report.daemon = Some(reply);
        }
        Ok(_) => {
            report.daemon = None;
        }
        Err(error) => {
            report.daemon = None;
            if report.control_socket.available {
                report.control_socket = CapabilityStatus::unavailable(
                    "control_socket",
                    format!("doctor failed: {}: {}", error.code, error.message),
                );
            }
        }
    }
}

pub fn print_platform_probe(report: &LaunchPlatformReport) {
    for status in report.statuses() {
        let marker = if status.available { "ok" } else { "unavailable" };
        println!("{}={} {}", status.name, marker, status.detail);
    }
    if let Some(daemon) = &report.daemon {
        println!(
            "{}",
            format_reply(&control_contract::reply::ControlReply::Doctor(
                daemon.clone(),
            ))
        );
    }
    println!("launch_seccomp_mode={}", recommended_seccomp_mode(report));
    if let Some(note) = recommended_launch_note(report) {
        println!("launch_note={note}");
    }
}

pub fn print_platform_probe_json(report: &LaunchPlatformReport) {
    let daemon = report.daemon.as_ref().map(|reply| {
        format!(
            "{{\"collectors\":[{}],\"plugins\":[{}],\"storage_ready\":{}}}",
            reply
                .available_collectors
                .iter()
                .map(|value| format!("\"{}\"", escape_json(value)))
                .collect::<Vec<_>>()
                .join(","),
            reply
                .loaded_policy_plugins
                .iter()
                .map(|value| format!("\"{}\"", escape_json(value)))
                .collect::<Vec<_>>()
                .join(","),
            reply.storage_ready
        )
    });
    let statuses = report
        .statuses()
        .iter()
        .map(|status| {
            format!(
                "{{\"name\":\"{}\",\"available\":{},\"detail\":\"{}\"}}",
                escape_json(status.name),
                status.available,
                escape_json(&status.detail)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"statuses\":[{}],\"recommended_seccomp_mode\":\"{}\",\"daemon\":{},\"launch_note\":\"{}\"}}",
        statuses,
        recommended_seccomp_mode(report),
        daemon.unwrap_or_else(|| "null".to_string()),
        escape_json(&recommended_launch_note(report).unwrap_or_default())
    );
}

impl LaunchPlatformReport {
    pub fn seccomp_launch_available(&self) -> bool {
        self.seccomp_launch.available
    }

    pub fn tls_sync_ready(&self) -> bool {
        self.tls_sync_runtime_library.available
            && (self.tls_sync_socket.available
                || self.tls_sync_socket.detail.starts_with("disabled"))
    }

    fn statuses(&self) -> Vec<&CapabilityStatus> {
        vec![
            &self.control_socket,
            &self.tls_sync_socket,
            &self.no_new_privs,
            &self.seccomp_launch,
            &self.tls_sync_runtime_library,
        ]
    }
}

pub fn recommended_seccomp_mode(report: &LaunchPlatformReport) -> &'static str {
    if report.seccomp_launch_available() {
        "require"
    } else {
        "skip"
    }
}

fn recommended_launch_note(report: &LaunchPlatformReport) -> Option<String> {
    if report.seccomp_launch_available() {
        return None;
    }
    if report.tls_sync_ready() {
        return Some(
            "seccomp launch path unavailable; use --seccomp-mode auto (default) for tls-sync-only launch"
                .to_string(),
        );
    }
    Some(
        "seccomp launch path unavailable and tls-sync prerequisites are incomplete; fix socket/runtime mounts before launch"
            .to_string(),
    )
}

pub fn probe_tls_sync_runtime_library(config: &PayloadTlsConfig) -> CapabilityStatus {
    if !config.enabled || !config.capture_backend.is_sync() {
        return CapabilityStatus::ok("tls_sync_runtime_library", "disabled by operator config");
    }
    let path = match tls_payload_sync::runtime_library_path(&match &config.sync_runtime_library_path
    {
        PayloadTlsSyncRuntimeLibraryPath::Auto => RuntimeLibraryPath::Auto,
        PayloadTlsSyncRuntimeLibraryPath::Path(path) => RuntimeLibraryPath::Path(path.clone()),
    }) {
        Ok(path) => path,
        Err(error) => return CapabilityStatus::unavailable("tls_sync_runtime_library", error.to_string()),
    };
    if path.is_file() {
        CapabilityStatus::ok(
            "tls_sync_runtime_library",
            format!("found {}", path.display()),
        )
    } else {
        CapabilityStatus::unavailable(
            "tls_sync_runtime_library",
            format!("missing {}", path.display()),
        )
    }
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_seccomp_mode_reflects_probe() {
        let available = LaunchPlatformReport {
            control_socket: CapabilityStatus::ok("control_socket", "ok"),
            tls_sync_socket: CapabilityStatus::ok("tls_sync_socket", "ok"),
            no_new_privs: CapabilityStatus::ok("no_new_privs", "ok"),
            seccomp_launch: CapabilityStatus::ok("seccomp_launch", "ok"),
            tls_sync_runtime_library: CapabilityStatus::ok("tls_sync_runtime_library", "ok"),
            daemon: None,
        };
        assert_eq!(recommended_seccomp_mode(&available), "require");

        let unavailable = LaunchPlatformReport {
            seccomp_launch: CapabilityStatus::unavailable("seccomp_launch", "denied"),
            ..available
        };
        assert_eq!(recommended_seccomp_mode(&unavailable), "skip");
    }
}

//! Local platform probes and launch recommendation for actrailctl.

use config_core::capture_profile::{CaptureProfile, DeploymentPermissions};
use config_core::daemon::{
    DisabledOrPath, EbpfEnabledMode, OperatorConfig, PayloadTlsConfig, PayloadTlsSeccompSyscall,
    PayloadTlsSyncRuntimeLibraryPath,
};
use control_contract::reply::DoctorReply;
use linux_platform::capability_probe::{CapabilityStatus, probe_no_new_privs, probe_unix_socket};
use model_core::capability::Capability;
use tls_payload_sync::RuntimeLibraryPath;

use crate::launch::controlled::ControlledChild;
use crate::launch::permission_policy::PermissionDecision;
use crate::launch::seccomp::SeccompSetup;
use crate::output::format_reply;
use crate::transport::ControlClientPort;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchPlatformReport {
    pub control_socket: CapabilityStatus,
    pub tls_sync_socket: CapabilityStatus,
    pub no_new_privs: CapabilityStatus,
    pub seccomp_notify: CapabilityStatus,
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
    let seccomp_notify =
        probe_seccomp_notify_capability(config.seccomp_notify.reserved_listener_fd);
    let tls_sync_runtime_library = probe_tls_sync_runtime_library(&config.payload_config.tls);
    LaunchPlatformReport {
        control_socket,
        tls_sync_socket,
        no_new_privs,
        seccomp_notify,
        tls_sync_runtime_library,
        daemon: None,
    }
}

pub fn probe_seccomp_notify_capability(reserved_listener_fd: u32) -> CapabilityStatus {
    let setup = match SeccompSetup::new(
        vec![PayloadTlsSeccompSyscall::Write],
        Vec::new(),
        4095,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        reserved_listener_fd,
    ) {
        Ok(setup) => setup,
        Err(error) => return CapabilityStatus::unavailable("seccomp_notify", error),
    };
    match ControlledChild::probe_seccomp_notify_path(&setup) {
        Ok(()) => CapabilityStatus::ok(
            "seccomp_notify",
            "seccomp user notify and pidfd_getfd launch path available",
        ),
        Err(error) => CapabilityStatus::unavailable("seccomp_notify", error),
    }
}

pub fn attach_daemon_status(
    report: &mut LaunchPlatformReport,
    client: &mut impl ControlClientPort,
    request_id: model_core::ids::RequestId,
) {
    match client.send(control_contract::command::ControlCommand::Doctor(
        control_contract::command::DoctorCommand { request_id },
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

pub fn print_platform_probe(report: &LaunchPlatformReport, decision: &PermissionDecision) {
    for status in report.statuses() {
        let marker = if status.available {
            "ok"
        } else {
            "unavailable"
        };
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
    print_permission_decision(decision);
    println!(
        "launch_seccomp_notify={}",
        enabled_disabled(decision.selected.seccomp_notify)
    );
    if let Some(note) = recommended_launch_note(report) {
        println!("launch_note={note}");
    }
}

pub fn print_permission_decision(decision: &PermissionDecision) {
    println!(
        "deployment_permissions_requested=host_ebpf:{},seccomp_notify:{}",
        decision.requested_host_ebpf.as_str(),
        decision.requested_seccomp_notify.as_str()
    );
    println!(
        "deployment_permissions_selected=host_ebpf:{},seccomp_notify:{}",
        enabled_disabled(decision.selected.host_ebpf),
        enabled_disabled(decision.selected.seccomp_notify)
    );
    println!("deployment_permissions_degraded={}", decision.degraded);
    println!(
        "deployment_required_capabilities={}",
        decision
            .required_capabilities
            .iter()
            .map(Capability::as_str)
            .collect::<Vec<_>>()
            .join(",")
    );
    if !decision.reasons.is_empty() {
        println!(
            "deployment_permission_reasons={}",
            decision.reasons.join("|")
        );
    }
}

pub fn print_platform_probe_json(report: &LaunchPlatformReport, decision: &PermissionDecision) {
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
    let required_capabilities = decision
        .required_capabilities
        .iter()
        .map(|capability| format!("\"{}\"", capability.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let reasons = decision
        .reasons
        .iter()
        .map(|reason| format!("\"{}\"", escape_json(reason)))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"statuses\":[{}],\"deployment_permissions\":{{\"requested\":{{\"host_ebpf\":\"{}\",\"seccomp_notify\":\"{}\"}},\"selected\":{{\"host_ebpf\":{},\"seccomp_notify\":{}}},\"degraded\":{},\"required_capabilities\":[{}],\"reasons\":[{}]}},\"launch_seccomp_notify\":{},\"daemon\":{},\"launch_note\":\"{}\"}}",
        statuses,
        decision.requested_host_ebpf.as_str(),
        decision.requested_seccomp_notify.as_str(),
        decision.selected.host_ebpf,
        decision.selected.seccomp_notify,
        decision.degraded,
        required_capabilities,
        reasons,
        decision.selected.seccomp_notify,
        daemon.unwrap_or_else(|| "null".to_string()),
        escape_json(&recommended_launch_note(report).unwrap_or_default())
    );
}

impl LaunchPlatformReport {
    pub fn seccomp_notify_available(&self) -> bool {
        self.seccomp_notify.available
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
            &self.seccomp_notify,
            &self.tls_sync_runtime_library,
        ]
    }
}

fn recommended_launch_note(report: &LaunchPlatformReport) -> Option<String> {
    if report.seccomp_notify_available() {
        return None;
    }
    if report.tls_sync_ready() {
        return Some(
            "seccomp-notify unavailable; use --seccomp-notify auto (default) to select a non-notify deployment"
                .to_string(),
        );
    }
    Some(
        "seccomp-notify unavailable and tls-sync prerequisites are incomplete; fix socket/runtime mounts before launch"
            .to_string(),
    )
}

fn enabled_disabled(value: bool) -> &'static str {
    if value { "enabled" } else { "disabled" }
}

pub fn probe_tls_sync_runtime_library(config: &PayloadTlsConfig) -> CapabilityStatus {
    if !config.enabled || !config.capture_backend.is_sync() {
        return CapabilityStatus::ok("tls_sync_runtime_library", "disabled by operator config");
    }
    let libraries = match tls_payload_sync::runtime_library_set(&match &config
        .sync_runtime_library_path
    {
        PayloadTlsSyncRuntimeLibraryPath::Auto => RuntimeLibraryPath::Auto,
        PayloadTlsSyncRuntimeLibraryPath::Path(path) => RuntimeLibraryPath::Path(path.clone()),
    }) {
        Ok(libraries) => libraries,
        Err(error) => {
            return CapabilityStatus::unavailable("tls_sync_runtime_library", error.to_string());
        }
    };
    let path = &libraries.glibc;
    if path.is_file() {
        let dependency_detail =
            match tls_payload_sync::runtime_dependency_report(std::slice::from_ref(&path)) {
                Ok(report) if report.guarded_libraries.is_empty() => {
                    "dependency_guard=none".to_string()
                }
                Ok(report) => format!(
                    "dependency_guard={}",
                    report
                        .guarded_libraries
                        .iter()
                        .map(|library| library.display().to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Err(error) => {
                    return CapabilityStatus::unavailable(
                        "tls_sync_runtime_library",
                        format!("{}; dependency guard failed: {error}", path.display()),
                    );
                }
            };
        let musl_detail = libraries
            .musl
            .as_ref()
            .map(|path| format!("musl_runtime={}", path.display()))
            .unwrap_or_else(|| "musl_runtime=missing".to_string());
        CapabilityStatus::ok(
            "tls_sync_runtime_library",
            format!(
                "glibc_runtime={}; {musl_detail}; {dependency_detail}",
                path.display()
            ),
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

/// Build a minimal usable operator config string tailored to the probe report.
///
/// Starts from the daemon's full default template and trims it down based on
/// what the probes found available. The result is printed to stdout so the
/// operator can redirect it to `/etc/actrail/actraild.conf` themselves. It is
/// intentionally stdout-only: `probe --suggest-config` never writes a file.
///
/// When `loaded` is `Some`, non-default socket/storage paths the operator
/// already chose are preserved (so re-suggesting does not clobber custom
/// paths). When `None` (no config exists yet, first deploy), template defaults
/// are used.
pub fn suggest_config_text(
    report: &LaunchPlatformReport,
    loaded: Option<&OperatorConfig>,
) -> String {
    let seccomp_available = report.seccomp_notify_available();
    let tls_sync_ready = report.tls_sync_ready();
    let ebpf_available = report
        .daemon
        .as_ref()
        .map(|reply| reply.available_collectors.iter().any(|c| c == "ebpf"));

    // Start from the default hierarchical template (or the loaded config, if
    // present, to preserve operator-chosen paths), parse it into an
    // OperatorConfig, mutate the fields the probes inform, and re-render to
    // TOML. This is far more robust than line-based patching of the template.
    let template = match loaded {
        Some(config) => match config.to_hierarchical_toml() {
            Ok(toml) => toml,
            Err(_) => OperatorConfig::default_hierarchical_template().unwrap_or_default(),
        },
        None => OperatorConfig::default_hierarchical_template().unwrap_or_default(),
    };
    let mut config = match OperatorConfig::parse(&template) {
        Ok(config) => config,
        Err(error) => {
            return format!("# suggest-config: could not parse baseline template: {error}\n");
        }
    };

    // ebpf: suggest "auto" so the daemon probes eBPF at startup and
    // auto-degrades (continuing without eBPF collection) when the host cannot
    // run eBPF — rather than refusing to start. The probe's `ebpf_available`
    // (from daemon doctor, when queried) is surfaced in the header comment
    // below; it does not change the suggestion since "auto" handles both
    // outcomes.
    config.ebpf_config.enabled_mode = EbpfEnabledMode::Auto;
    // `enabled` is the daemon-resolved effective flag; at config-suggestion
    // time we have not run the host probe, so leave it false (matches the
    // parse-time default for Auto). The daemon will set it at startup.
    config.ebpf_config.enabled = false;

    // TLS plaintext capture: only when the tls-sync prerequisites are met.
    if !tls_sync_ready {
        config.payload_config.tls.enabled = false;
    }
    // payload_tls_binary_path MUST stay disabled under tls-sync (the sync
    // backend builds the probe plan dynamically at launch). Enforce it
    // regardless of probe results — a fixed path is rejected with
    // "tls-sync auto plan requires payload_tls_binary_path=disabled".
    config.payload_config.tls.binary_path = DisabledOrPath::Disabled;

    // Launch-time process seccomp: only when the seccomp launch path is
    // actually usable. When unavailable (e.g. Docker default seccomp blocks
    // pidfd_getfd), disable it and drop the capabilities that require it so
    // the daemon starts without requiring proc-exec-context.
    if !seccomp_available {
        config.process_seccomp.enabled = false;
    }
    trim_profile_to_available_permissions(
        &mut config.capture_profile,
        DeploymentPermissions::new(ebpf_available.unwrap_or(true), seccomp_available),
    );

    let body = config
        .to_hierarchical_toml()
        .unwrap_or_else(|error| format!("# suggest-config: could not render config: {error}\n"));
    let header = suggest_config_header(
        report,
        seccomp_available,
        tls_sync_ready,
        ebpf_available.unwrap_or(false),
    );
    format!("{header}\n{body}")
}

fn trim_profile_to_available_permissions(
    profile: &mut CaptureProfile,
    permissions: DeploymentPermissions,
) {
    let name = profile.name.clone();
    *profile = profile.for_permissions(permissions);
    profile.name = name;
}

fn suggest_config_header(
    report: &LaunchPlatformReport,
    seccomp_available: bool,
    tls_sync_ready: bool,
    ebpf_available: bool,
) -> String {
    let mut lines = Vec::new();
    lines.push(
        "# AcTrail operator config — suggested by `actrailctl probe --suggest-config`.".into(),
    );
    lines.push("# This is a starting point trimmed to what the probes found available.".into());
    lines.push("# Review it before use, then: actrailctl probe --suggest-config > /etc/actrail/actraild.conf".into());
    lines.push("# (or redirect to a temp file and install -m 0644 it into place).".into());
    lines.push(String::new());
    lines.push("# Probe summary:".into());
    lines.push(format!(
        "#   control_socket        = {}",
        capability_summary(&report.control_socket)
    ));
    lines.push(format!(
        "#   tls_sync_socket       = {}",
        capability_summary(&report.tls_sync_socket)
    ));
    lines.push(format!(
        "#   no_new_privs          = {}",
        capability_summary(&report.no_new_privs)
    ));
    lines.push(format!(
        "#   seccomp_notify        = {} ({}process seccomp {})",
        capability_summary(&report.seccomp_notify),
        if seccomp_available { "" } else { "→ " },
        if seccomp_available {
            "enabled"
        } else {
            "disabled in suggested config"
        }
    ));
    lines.push(format!(
        "#   tls_sync_runtime_lib  = {} (TLS plaintext capture {})",
        capability_summary(&report.tls_sync_runtime_library),
        if tls_sync_ready {
            "enabled"
        } else {
            "disabled in suggested config"
        }
    ));
    if let Some(daemon) = &report.daemon {
        lines.push(format!(
            "#   daemon collectors     = {} (ebpf {}; [ebpf] enabled=\"auto\" probes at startup)",
            daemon.available_collectors.join(","),
            if ebpf_available {
                "present"
            } else {
                "absent — host eBPF unavailable, auto will degrade"
            }
        ));
    } else {
        lines.push("#   daemon                = not queried (--skip-daemon); [ebpf] enabled=\"auto\" probes at startup".into());
    }
    lines.join("\n")
}

fn capability_summary(status: &CapabilityStatus) -> String {
    if status.available {
        "ok".to_string()
    } else {
        "unavailable".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_ok_report() -> LaunchPlatformReport {
        LaunchPlatformReport {
            control_socket: CapabilityStatus::ok("control_socket", "ok"),
            tls_sync_socket: CapabilityStatus::ok("tls_sync_socket", "ok"),
            no_new_privs: CapabilityStatus::ok("no_new_privs", "ok"),
            seccomp_notify: CapabilityStatus::ok("seccomp_notify", "ok"),
            tls_sync_runtime_library: CapabilityStatus::ok("tls_sync_runtime_library", "ok"),
            daemon: None,
        }
    }

    #[test]
    fn suggest_config_seccomp_unavailable_disables_process_seccomp() {
        let report = LaunchPlatformReport {
            seccomp_notify: CapabilityStatus::unavailable("seccomp_notify", "denied"),
            ..all_ok_report()
        };
        let config = suggest_config_text(&report, None);
        // Hierarchical TOML: [process_seccomp] enabled = false
        assert!(
            config.contains("[process_seccomp]") && config.contains("enabled = false"),
            "process_seccomp must be disabled when seccomp unavailable"
        );
        // proc-exec-context capability must be dropped from [capture].
        let parsed = OperatorConfig::parse(&config).expect("suggested config parses");
        assert!(
            !parsed
                .capture_profile
                .capabilities
                .iter()
                .any(|r| r.capability == model_core::capability::Capability::ProcExecContext),
            "proc-exec-context must be dropped when seccomp unavailable"
        );
    }

    #[test]
    fn suggest_config_seccomp_available_keeps_process_seccomp() {
        let config = suggest_config_text(&all_ok_report(), None);
        let parsed = OperatorConfig::parse(&config).expect("suggested config parses");
        assert!(
            parsed.process_seccomp.enabled,
            "process_seccomp stays enabled when seccomp available"
        );
    }

    #[test]
    fn suggest_config_uses_permission_truth_table_for_missing_ebpf() {
        let report = LaunchPlatformReport {
            daemon: Some(DoctorReply {
                available_collectors: vec!["tls-sync".to_string()],
                loaded_policy_plugins: Vec::new(),
                storage_ready: true,
            }),
            ..all_ok_report()
        };
        let config = suggest_config_text(&report, None);
        let parsed = OperatorConfig::parse(&config).expect("suggested config parses");

        assert!(
            !parsed
                .capture_profile
                .capabilities
                .iter()
                .any(|request| { request.capability == Capability::SocketPlaintextPayload })
        );
        assert!(
            parsed
                .capture_profile
                .capabilities
                .iter()
                .any(|request| { request.capability == Capability::ResourceMetrics })
        );
    }

    #[test]
    fn suggest_config_tls_sync_unavailable_disables_payload_tls() {
        let report = LaunchPlatformReport {
            tls_sync_socket: CapabilityStatus::unavailable("tls_sync_socket", "no socket"),
            tls_sync_runtime_library: CapabilityStatus::unavailable(
                "tls_sync_runtime_library",
                "missing",
            ),
            ..all_ok_report()
        };
        let config = suggest_config_text(&report, None);
        let parsed = OperatorConfig::parse(&config).expect("suggested config parses");
        assert!(
            !parsed.payload_config.tls.enabled,
            "payload_tls must be disabled when tls-sync prerequisites are missing"
        );
    }

    #[test]
    fn suggest_config_always_keeps_binary_path_disabled() {
        // Regardless of probe results, payload_tls_binary_path must stay
        // disabled under the tls-sync backend.
        let config = suggest_config_text(&all_ok_report(), None);
        let parsed = OperatorConfig::parse(&config).expect("suggested config parses");
        assert!(matches!(
            parsed.payload_config.tls.binary_path,
            config_core::daemon::DisabledOrPath::Disabled
        ));
    }

    #[test]
    fn suggest_config_no_loaded_config_parses_as_valid() {
        // With no existing config, the suggestion must still be a valid
        // operator config (round-trip through OperatorConfig::parse). The
        // header comment lines start with `#` and are ignored by the parser.
        let config = suggest_config_text(&all_ok_report(), None);
        OperatorConfig::parse(&config).expect("suggested config parses as valid OperatorConfig");
    }

    #[test]
    fn suggest_config_sets_ebpf_auto() {
        // The suggested config should set [ebpf] enabled = "auto" so the
        // daemon probes eBPF at startup and auto-degrades when the host
        // cannot run it, regardless of the current probe result.
        let config = suggest_config_text(&all_ok_report(), None);
        let parsed = OperatorConfig::parse(&config).expect("suggested config parses");
        assert_eq!(
            parsed.ebpf_config.enabled_mode,
            config_core::daemon::EbpfEnabledMode::Auto
        );
        // enabled is the daemon-resolved flag; at suggestion time it stays
        // false for Auto (the daemon sets it at startup).
        assert!(!parsed.ebpf_config.enabled);
    }
}

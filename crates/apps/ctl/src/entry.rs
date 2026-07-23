//! Top-level entry boundary for the control application.

use std::path::Path;

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};
use control_contract::command::{ControlCommand, ResolveLaunchPermissionsCommand};
use control_contract::reply::{ControlReply, DoctorReply};
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use crate::args::{CtlCommand, parse_args};
use crate::clean::run_clean;
use crate::dispatch::dispatch;
use crate::launch::permission_policy::{
    contract_permission_mode, permission_decision_from_reply, resolve_deployment_permissions,
};
use crate::launch::{LaunchRequest, run_launch};
use crate::output::format_reply;
use crate::platform_probe::{
    LaunchPlatformReport, attach_daemon_status, print_platform_probe, print_platform_probe_json,
    run_platform_probe, suggest_config_text,
};

pub fn run_from_env() -> Result<i32, String> {
    let invocation = parse_args(std::env::args().skip(1))?;
    match invocation.command {
        CtlCommand::Init {
            config_path,
            force,
            patch_path,
        } => {
            match initialize_operator_config_file(&config_path, force, patch_path.as_deref())? {
                OperatorConfigInitStatus::Created => {
                    println!("initialized config {}", config_path.display());
                }
                OperatorConfigInitStatus::ExistingValid => {
                    println!(
                        "config {} already exists and is valid",
                        config_path.display()
                    );
                }
                OperatorConfigInitStatus::Overwritten => {
                    println!("overwrote config {}", config_path.display());
                }
            }
            Ok(i32::default())
        }
        CtlCommand::Clean { artifacts } => run_clean(artifacts),
        CtlCommand::Launch {
            display_name,
            capture_profile,
            tags,
            payload_tls_config,
            payload_tls_seccomp_syscalls,
            payload_socket_seccomp_syscalls,
            payload_socket_max_segment_bytes,
            process_seccomp_syscalls,
            network_control_syscalls,
            file_enforcement_syscalls,
            seccomp_notify_reserved_listener_fd,
            agent_invocation_commands,
            supervision_poll_interval_ms,
            ebpf_seccomp_policy,
            argv,
        } => {
            let socket_path = required_socket_path(invocation.socket_path)?;
            let transport = UdsSocketTransport::new(socket_path.clone());
            let mut client = UdsControlClient::new(transport);
            run_launch(
                &mut client,
                invocation.request_id,
                LaunchRequest {
                    control_socket_path: socket_path,
                    display_name,
                    capture_profile,
                    tags,
                    payload_tls_config,
                    payload_tls_seccomp_syscalls,
                    payload_socket_seccomp_syscalls,
                    payload_socket_max_segment_bytes,
                    process_seccomp_syscalls,
                    network_control_syscalls,
                    file_enforcement_syscalls,
                    seccomp_notify_reserved_listener_fd,
                    agent_invocation_commands,
                    supervision_poll_interval_ms,
                    ebpf_seccomp_policy,
                    argv,
                },
            )
        }
        CtlCommand::Probe {
            operator_config,
            json,
            skip_daemon,
            suggest_config,
            ebpf_seccomp_policy,
        } => {
            // For --suggest-config, probe must work without an existing config;
            // build a minimal report from defaults when none was loaded.
            let fallback_default = match operator_config.as_ref() {
                None => Some(OperatorConfig::parse(
                    &OperatorConfig::default_hierarchical_template()
                        .map_err(|error| format!("render default template: {error}"))?,
                )?),
                Some(_) => None,
            };
            let loaded = operator_config
                .as_ref()
                .or(fallback_default.as_ref())
                .expect("loaded or fallback default is present");
            let mut report = run_platform_probe(loaded);
            // For --suggest-config with no config, socket_path may be None;
            // daemon query is best-effort then. Otherwise (--skip-daemon or
            // normal probe) honor the explicit skip or require the socket.
            let daemon_socket = if skip_daemon {
                None
            } else {
                match required_socket_path(invocation.socket_path.clone()) {
                    Ok(path) => Some(path),
                    Err(_) if suggest_config => None,
                    Err(error) => return Err(error),
                }
            };
            let daemon_decision = if let Some(socket_path) = daemon_socket {
                let transport = UdsSocketTransport::new(socket_path);
                let mut client = UdsControlClient::new(transport);
                attach_daemon_status(&mut report, &mut client, invocation.request_id);
                if suggest_config {
                    None
                } else {
                    let reply = client
                        .send(ControlCommand::ResolveLaunchPermissions(
                            ResolveLaunchPermissionsCommand {
                                request_id: invocation.request_id,
                                profile_name: loaded.capture_profile.name.clone(),
                                host_ebpf: contract_permission_mode(ebpf_seccomp_policy.host_ebpf),
                                seccomp_notify: contract_permission_mode(
                                    ebpf_seccomp_policy.seccomp_notify,
                                ),
                                seccomp_notify_available: report.seccomp_notify_available(),
                                seccomp_notify_detail: report.seccomp_notify.detail.clone(),
                            },
                        ))
                        .map_err(|error| {
                            format!(
                                "resolve launch permissions failed: {}: {}",
                                error.code, error.message
                            )
                        })?;
                    let ControlReply::LaunchPermissions(reply) = reply else {
                        return Err(
                            "resolve launch permissions returned unexpected reply".to_string()
                        );
                    };
                    Some(permission_decision_from_reply(&reply))
                }
            } else {
                None
            };
            if suggest_config {
                print!("{}", suggest_config_text(&report, operator_config.as_ref()));
                return Ok(i32::default());
            }
            let launch_seccomp_requirements = loaded.launch_seccomp_requirements();
            let decision = match daemon_decision {
                Some(decision) => decision,
                None => {
                    let local_preview = LaunchPlatformReport {
                        daemon: Some(DoctorReply {
                            available_collectors: Vec::new(),
                            loaded_policy_plugins: Vec::new(),
                            storage_ready: false,
                        }),
                        ..report.clone()
                    };
                    resolve_deployment_permissions(
                        ebpf_seccomp_policy,
                        &loaded.capture_profile,
                        launch_seccomp_requirements,
                        Some(&local_preview),
                    )?
                }
            };
            if json {
                print_platform_probe_json(&report, &decision);
            } else {
                print_platform_probe(&report, &decision);
            }
            Ok(i32::default())
        }
        command => {
            let transport = UdsSocketTransport::new(required_socket_path(invocation.socket_path)?);
            let mut client = UdsControlClient::new(transport);
            let reply = dispatch(&mut client, invocation.request_id, command).map_err(|error| {
                format!("control command failed: {}: {}", error.code, error.message)
            })?;
            println!("{}", format_reply(&reply));
            Ok(i32::default())
        }
    }
}

fn initialize_operator_config_file(
    path: &Path,
    force: bool,
    patch_path: Option<&Path>,
) -> Result<OperatorConfigInitStatus, String> {
    let existed = path.exists();
    if existed && !force {
        if let Some(patch_path) = patch_path {
            return Err(format!(
                "config {} already exists; pass --force to rewrite it with patch {}",
                path.display(),
                patch_path.display()
            ));
        }
        OperatorConfig::load(path)
            .map_err(|error| format!("validate config {}: {error}", path.display()))?;
        return Ok(OperatorConfigInitStatus::ExistingValid);
    }
    let mut config = OperatorConfig::init()?;
    if let Some(patch_path) = patch_path {
        config = config.patch_file(patch_path)?;
    }
    config.dump_to_path(path, force)?;
    Ok(if existed {
        OperatorConfigInitStatus::Overwritten
    } else {
        OperatorConfigInitStatus::Created
    })
}

fn required_socket_path(
    socket_path: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    socket_path.ok_or_else(|| "missing control socket path".to_string())
}

//! Top-level entry boundary for the control application.

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use crate::args::{CtlCommand, parse_args};
use crate::clean::run_clean;
use crate::dispatch::dispatch;
use crate::launch::{LaunchRequest, run_launch};
use crate::output::format_reply;
use crate::platform_probe::{
    attach_daemon_status, print_platform_probe, print_platform_probe_json, run_platform_probe,
    suggest_config_text,
};

pub fn run_from_env() -> Result<i32, String> {
    let invocation = parse_args(std::env::args().skip(1))?;
    match invocation.command {
        CtlCommand::Init { config_path, force } => {
            match OperatorConfig::initialize(&config_path, force)? {
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
            profile_name,
            tags,
            payload_tls_enabled,
            payload_tls_config,
            payload_tls_seccomp_syscalls,
            payload_socket_enabled,
            payload_socket_seccomp_syscalls,
            payload_socket_max_segment_bytes,
            process_seccomp_enabled,
            process_seccomp_syscalls,
            seccomp_notify_reserved_listener_fd,
            agent_invocation_commands,
            seccomp_mode,
            argv,
        } => {
            let transport = UdsSocketTransport::new(required_socket_path(invocation.socket_path)?);
            let mut client = UdsControlClient::new(transport);
            run_launch(
                &mut client,
                invocation.request_id,
                LaunchRequest {
                    display_name,
                    profile_name,
                    tags,
                    payload_tls_enabled,
                    payload_tls_config,
                    payload_tls_seccomp_syscalls,
                    payload_socket_enabled,
                    payload_socket_seccomp_syscalls,
                    payload_socket_max_segment_bytes,
                    process_seccomp_enabled,
                    process_seccomp_syscalls,
                    seccomp_notify_reserved_listener_fd,
                    agent_invocation_commands,
                    seccomp_mode,
                    argv,
                },
            )
        }
        CtlCommand::Probe {
            operator_config,
            json,
            skip_daemon,
            suggest_config,
        } => {
            // For --suggest-config, probe must work without an existing config;
            // build a minimal report from defaults when none was loaded.
            let default_config = || -> Result<OperatorConfig, String> {
                OperatorConfig::parse(config_core::daemon::OPERATOR_CONFIG_TEMPLATE)
                    .map_err(|error| format!("parse default template: {error}"))
            };
            let loaded = match &operator_config {
                Some(config) => config,
                None => &default_config()?,
            };
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
            if let Some(socket_path) = daemon_socket {
                let transport = UdsSocketTransport::new(socket_path);
                let mut client = UdsControlClient::new(transport);
                attach_daemon_status(&mut report, &mut client);
            }
            if suggest_config {
                print!("{}", suggest_config_text(&report, operator_config.as_ref()));
                return Ok(i32::default());
            }
            if json {
                print_platform_probe_json(&report);
            } else {
                print_platform_probe(&report);
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

fn required_socket_path(
    socket_path: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    socket_path.ok_or_else(|| "missing control socket path".to_string())
}

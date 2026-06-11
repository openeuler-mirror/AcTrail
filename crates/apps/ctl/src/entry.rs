//! Top-level entry boundary for the control application.

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use crate::args::{CtlCommand, parse_args};
use crate::clean::run_clean;
use crate::dispatch::dispatch;
use crate::launch::{LaunchRequest, run_launch};
use crate::output::format_reply;

pub fn run_from_env() -> Result<i32, String> {
    let invocation = parse_args(std::env::args().skip(1))?;
    match invocation.command {
        CtlCommand::Init { config_path } => {
            match OperatorConfig::initialize(&config_path)? {
                OperatorConfigInitStatus::Created => {
                    println!("initialized config {}", config_path.display());
                }
                OperatorConfigInitStatus::ExistingValid => {
                    println!(
                        "config {} already exists and is valid",
                        config_path.display()
                    );
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
                    argv,
                },
            )
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

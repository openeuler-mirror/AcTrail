//! Top-level entry boundary for the control application.

use uds_control_client::{UdsControlClient, UdsSocketTransport};

use crate::args::{CtlCommand, parse_args};
use crate::clean::run_clean;
use crate::dispatch::dispatch;
use crate::launch::{LaunchRequest, run_launch};
use crate::output::format_reply;

pub fn run_from_env() -> Result<i32, String> {
    let invocation = parse_args(std::env::args().skip(1))?;
    match invocation.command {
        CtlCommand::Clean { artifacts } => run_clean(artifacts),
        CtlCommand::Launch {
            display_name,
            profile_name,
            tags,
            payload_tls_enabled,
            payload_tls_seccomp_syscalls,
            payload_socket_enabled,
            payload_socket_seccomp_syscalls,
            payload_socket_max_segment_bytes,
            process_seccomp_enabled,
            process_seccomp_syscalls,
            seccomp_notify_reserved_listener_fd,
            argv,
        } => {
            let transport = UdsSocketTransport::new(invocation.socket_path);
            let mut client = UdsControlClient::new(transport);
            run_launch(
                &mut client,
                invocation.request_id,
                LaunchRequest {
                    display_name,
                    profile_name,
                    tags,
                    payload_tls_enabled,
                    payload_tls_seccomp_syscalls,
                    payload_socket_enabled,
                    payload_socket_seccomp_syscalls,
                    payload_socket_max_segment_bytes,
                    process_seccomp_enabled,
                    process_seccomp_syscalls,
                    seccomp_notify_reserved_listener_fd,
                    argv,
                },
            )
        }
        command => {
            let transport = UdsSocketTransport::new(invocation.socket_path);
            let mut client = UdsControlClient::new(transport);
            let reply = dispatch(&mut client, invocation.request_id, command).map_err(|error| {
                format!("control command failed: {}: {}", error.code, error.message)
            })?;
            println!("{}", format_reply(&reply));
            Ok(i32::default())
        }
    }
}

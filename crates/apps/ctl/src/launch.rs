//! Launch command orchestration.

#[path = "launch/seccomp.rs"]
mod seccomp;
#[path = "launch/sync.rs"]
mod sync;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::{Command, ExitStatus};

use config_core::daemon::{
    PayloadSocketSeccompSyscall, PayloadTlsConfig, PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use control_contract::command::{ControlCommand, TrackAddCommand, TrackRemoveCommand};
use control_contract::reply::{ControlError, ControlReply};
use control_contract::selector::TraceSelector;
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};

use crate::output::format_reply;
use crate::transport::ControlClientPort;
use seccomp::run_child_seccomp;
use sync::{run_child_sync_tls, sync_launch};

pub(crate) struct LaunchRequest {
    pub display_name: TraceName,
    pub profile_name: ProfileName,
    pub tags: BTreeSet<String>,
    pub payload_tls_enabled: bool,
    pub payload_tls_config: PayloadTlsConfig,
    pub payload_tls_seccomp_syscalls: Vec<PayloadTlsSeccompSyscall>,
    pub payload_socket_enabled: bool,
    pub payload_socket_seccomp_syscalls: Vec<PayloadSocketSeccompSyscall>,
    pub payload_socket_max_segment_bytes: u32,
    pub process_seccomp_enabled: bool,
    pub process_seccomp_syscalls: Vec<ProcessSeccompSyscall>,
    pub seccomp_notify_reserved_listener_fd: u32,
    pub agent_invocation_commands: Vec<String>,
    pub argv: Vec<String>,
}

pub(crate) fn run_launch(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    request: LaunchRequest,
) -> Result<i32, String> {
    let reply = client
        .send(ControlCommand::TrackAdd(TrackAddCommand {
            request_id,
            root_pid: std::process::id(),
            display_name: request.display_name,
            profile_name: request.profile_name,
            tags: request.tags,
            launch_mode: true,
        }))
        .map_err(format_control_error)?;
    let trace_id = track_added_trace_id(&reply)?;
    println!("{}", format_reply(&reply));
    let tls_sync_enabled =
        request.payload_tls_config.enabled && request.payload_tls_config.capture_backend.is_sync();
    let seccomp_enabled = request.payload_tls_enabled
        || request.payload_socket_enabled
        || request.process_seccomp_enabled;
    let child_result = if tls_sync_enabled && seccomp_enabled {
        let launch = sync_launch(
            trace_id,
            request.argv,
            &request.payload_tls_config,
            &request.agent_invocation_commands,
        )?;
        let payload_tls_seccomp_syscalls = Vec::new();
        let payload_socket_seccomp_syscalls = if request.payload_socket_enabled {
            request.payload_socket_seccomp_syscalls
        } else {
            Vec::new()
        };
        let process_seccomp_syscalls = if request.process_seccomp_enabled {
            request.process_seccomp_syscalls
        } else {
            Vec::new()
        };
        run_child_seccomp(
            client,
            next_request_id(request_id)?,
            trace_id,
            launch.command,
            payload_tls_seccomp_syscalls,
            payload_socket_seccomp_syscalls,
            request.payload_socket_max_segment_bytes,
            process_seccomp_syscalls,
            request.seccomp_notify_reserved_listener_fd,
            launch.envs,
        )
    } else if tls_sync_enabled {
        run_child_sync_tls(
            trace_id,
            request.argv,
            &request.payload_tls_config,
            &request.agent_invocation_commands,
        )
    } else if seccomp_enabled {
        let payload_tls_seccomp_syscalls = if request.payload_tls_enabled {
            request.payload_tls_seccomp_syscalls
        } else {
            Vec::new()
        };
        let payload_socket_seccomp_syscalls = if request.payload_socket_enabled {
            request.payload_socket_seccomp_syscalls
        } else {
            Vec::new()
        };
        let process_seccomp_syscalls = if request.process_seccomp_enabled {
            request.process_seccomp_syscalls
        } else {
            Vec::new()
        };
        run_child_seccomp(
            client,
            next_request_id(request_id)?,
            trace_id,
            request.argv.into_iter().map(OsString::from).collect(),
            payload_tls_seccomp_syscalls,
            payload_socket_seccomp_syscalls,
            request.payload_socket_max_segment_bytes,
            process_seccomp_syscalls,
            request.seccomp_notify_reserved_listener_fd,
            Vec::new(),
        )
    } else {
        run_child(request.argv)
    };
    remove_launch_root(
        client,
        launch_remove_request_id(request_id, seccomp_enabled)?,
        trace_id,
    )?;
    child_result
}

fn run_child(argv: Vec<String>) -> Result<i32, String> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "launch requires a command after --".to_string())?;
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("launch child {program}: {error}"))?;
    exit_code(status)
}

fn track_added_trace_id(reply: &ControlReply) -> Result<TraceId, String> {
    match reply {
        ControlReply::TrackAdded(reply) => Ok(reply.trace_id),
        _ => Err("track add returned unexpected reply".to_string()),
    }
}

fn next_request_id(request_id: RequestId) -> Result<RequestId, String> {
    request_id
        .get()
        .checked_add(1)
        .map(RequestId::new)
        .ok_or_else(|| "request id overflow".to_string())
}

fn launch_remove_request_id(
    request_id: RequestId,
    seccomp_enabled: bool,
) -> Result<RequestId, String> {
    let mut next = next_request_id(request_id)?;
    if seccomp_enabled {
        next = next_request_id(next)?;
    }
    Ok(next)
}

fn remove_launch_root(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
) -> Result<(), String> {
    client
        .send(ControlCommand::TrackRemove(TrackRemoveCommand {
            request_id,
            selector: TraceSelector::TraceId(trace_id),
        }))
        .map(|_| ())
        .map_err(format_control_error)
}

fn exit_code(status: ExitStatus) -> Result<i32, String> {
    status
        .code()
        .ok_or_else(|| "launch child terminated without an exit code".to_string())
}

fn format_control_error(error: ControlError) -> String {
    format!("control command failed: {}: {}", error.code, error.message)
}

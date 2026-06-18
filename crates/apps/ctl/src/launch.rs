//! Launch command orchestration.

#[path = "launch/controlled.rs"]
mod controlled;
#[path = "launch/java_agent.rs"]
mod java_agent;
#[path = "launch/seccomp.rs"]
mod seccomp;
#[path = "launch/suppress.rs"]
mod suppress;
#[path = "launch/sync.rs"]
mod sync;

use std::collections::BTreeSet;
use std::ffi::OsString;

use config_core::daemon::{
    PayloadSocketSeccompSyscall, PayloadTlsConfig, PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use control_contract::command::{ControlCommand, TrackAddCommand, TrackRemoveCommand};
use control_contract::reply::{ControlError, ControlReply};
use control_contract::selector::TraceSelector;
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};
use model_core::process::SuppressedFdPurpose;

use crate::output::format_reply;
use crate::transport::ControlClientPort;
use controlled::{ChildSetup, ControlledChild};
use seccomp::{SeccompSetup, register_listener};
use suppress::InheritableSuppressedFd;
use sync::{SyncLaunch, sync_launch, sync_launch_envs};

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
    let tls_sync_enabled =
        request.payload_tls_config.enabled && request.payload_tls_config.capture_backend.is_sync();
    let payload_tls_seccomp_enabled = request.payload_tls_enabled && !tls_sync_enabled;
    let seccomp_enabled = payload_tls_seccomp_enabled
        || request.payload_socket_enabled
        || request.process_seccomp_enabled;
    let child_setup = if seccomp_enabled {
        ChildSetup::Seccomp(seccomp_setup(&request, payload_tls_seccomp_enabled)?)
    } else {
        ChildSetup::Plain
    };

    let raw_argv = request.argv;
    let sync_launch = if tls_sync_enabled {
        Some(sync_launch(
            raw_argv.clone(),
            &request.payload_tls_config,
            &request.agent_invocation_commands,
        )?)
    } else {
        None
    };
    let command = sync_launch
        .as_ref()
        .map(|launch| launch.command.clone())
        .unwrap_or_else(|| raw_argv.into_iter().map(OsString::from).collect());
    let mut sync_event_fd = if tls_sync_enabled {
        Some(InheritableSuppressedFd::connect_unix_socket(
            &request.payload_tls_config.sync_event_socket_path,
            SuppressedFdPurpose::TlsSyncEvent,
        )?)
    } else {
        None
    };
    let mut child = ControlledChild::spawn(command, child_setup)?;

    let reply = match client.send(ControlCommand::TrackAdd(TrackAddCommand {
        request_id,
        root_pid: child.pid(),
        display_name: request.display_name,
        profile_name: request.profile_name,
        tags: request.tags,
        launch_mode: true,
        initial_suppressed_fds: sync_event_fd
            .as_ref()
            .map(InheritableSuppressedFd::initial_suppressed_fd)
            .into_iter()
            .collect(),
    })) {
        Ok(reply) => reply,
        Err(error) => {
            child.terminate();
            return Err(format_control_error(error));
        }
    };
    let trace_id = match track_added_trace_id(&reply) {
        Ok(trace_id) => trace_id,
        Err(error) => {
            child.terminate();
            return Err(error);
        }
    };
    println!("{}", format_reply(&reply));

    if let Err(error) =
        register_seccomp_listener_if_needed(client, request_id, trace_id, seccomp_enabled, &child)
    {
        child.terminate();
        remove_launch_root_best_effort(
            client,
            launch_remove_request_id(request_id, seccomp_enabled)?,
            trace_id,
        );
        return Err(error);
    }
    let envs = match launch_envs(
        trace_id,
        &request.payload_tls_config,
        request.payload_socket_max_segment_bytes,
        sync_launch.as_ref(),
        sync_event_fd.as_ref(),
    ) {
        Ok(envs) => envs,
        Err(error) => {
            child.terminate();
            remove_launch_root_best_effort(
                client,
                launch_remove_request_id(request_id, seccomp_enabled)?,
                trace_id,
            );
            return Err(error);
        }
    };
    drop(sync_event_fd.take());
    if let Err(error) = child.continue_with_envs(envs) {
        child.terminate();
        remove_launch_root_best_effort(
            client,
            launch_remove_request_id(request_id, seccomp_enabled)?,
            trace_id,
        );
        return Err(error);
    }
    let child_result = child.wait();
    remove_launch_root(
        client,
        launch_remove_request_id(request_id, seccomp_enabled)?,
        trace_id,
    )?;
    child_result
}

fn seccomp_setup(
    request: &LaunchRequest,
    payload_tls_seccomp_enabled: bool,
) -> Result<SeccompSetup, String> {
    let payload_tls_seccomp_syscalls = if payload_tls_seccomp_enabled {
        request.payload_tls_seccomp_syscalls.clone()
    } else {
        Vec::new()
    };
    let payload_socket_seccomp_syscalls = if request.payload_socket_enabled {
        request.payload_socket_seccomp_syscalls.clone()
    } else {
        Vec::new()
    };
    let process_seccomp_syscalls = if request.process_seccomp_enabled {
        request.process_seccomp_syscalls.clone()
    } else {
        Vec::new()
    };
    SeccompSetup::new(
        payload_tls_seccomp_syscalls,
        payload_socket_seccomp_syscalls,
        request.payload_socket_max_segment_bytes,
        process_seccomp_syscalls,
        request.seccomp_notify_reserved_listener_fd,
    )
}

fn register_seccomp_listener_if_needed(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
    seccomp_enabled: bool,
    child: &ControlledChild,
) -> Result<(), String> {
    if !seccomp_enabled {
        return Ok(());
    }
    let listener_fd = child
        .listener_fd()
        .ok_or_else(|| "seccomp launch child did not expose a listener fd".to_string())?;
    register_listener(
        client,
        next_request_id(request_id)?,
        trace_id,
        child.pid(),
        listener_fd,
    )
}

fn launch_envs(
    trace_id: TraceId,
    payload_tls_config: &PayloadTlsConfig,
    payload_socket_max_segment_bytes: u32,
    sync_launch: Option<&SyncLaunch>,
    sync_event_fd: Option<&InheritableSuppressedFd>,
) -> Result<Vec<(OsString, OsString)>, String> {
    match sync_launch {
        Some(sync_launch) => sync_launch_envs(
            trace_id,
            payload_tls_config,
            payload_socket_max_segment_bytes,
            sync_launch,
            sync_event_fd,
        ),
        None => Ok(Vec::new()),
    }
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

fn remove_launch_root_best_effort(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
) {
    let _ = remove_launch_root(client, request_id, trace_id);
}

fn format_control_error(error: ControlError) -> String {
    format!("control command failed: {}: {}", error.code, error.message)
}

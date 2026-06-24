//! Launch command orchestration.

#[path = "launch/controlled.rs"]
pub(crate) mod controlled;
#[path = "launch/java_agent.rs"]
mod java_agent;
#[path = "launch/seccomp.rs"]
pub(crate) mod seccomp;
#[path = "launch/seccomp_mode.rs"]
pub(crate) mod seccomp_mode;
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
use crate::process_ref::process_ref;
use crate::transport::ControlClientPort;
use controlled::{ChildSetup, ControlledChild};
use seccomp::{SeccompSetup, register_listener};
use seccomp_mode::{LaunchSeccompMode, resolve_launch_seccomp};
use suppress::InheritableSuppressedFd;
use sync::{SyncLaunch, sync_launch, sync_launch_envs};

use crate::platform_probe::{
    LaunchPlatformReport, probe_seccomp_launch_capability, probe_tls_sync_runtime_library,
};
use linux_platform::capability_probe::{probe_no_new_privs, probe_unix_socket};

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
    pub seccomp_mode: LaunchSeccompMode,
    pub argv: Vec<String>,
}

pub(crate) fn run_launch(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    request: LaunchRequest,
) -> Result<i32, String> {
    let tls_sync_enabled =
        request.payload_tls_config.enabled && request.payload_tls_config.capture_backend.is_sync();
    let payload_tls_seccomp_configured = request.payload_tls_enabled && !tls_sync_enabled;
    let probe = if matches!(request.seccomp_mode, LaunchSeccompMode::Auto | LaunchSeccompMode::Require) {
        Some(run_platform_probe_from_launch(&request))
    } else {
        None
    };
    let effective = resolve_launch_seccomp(
        request.seccomp_mode,
        tls_sync_enabled,
        payload_tls_seccomp_configured,
        request.payload_socket_enabled,
        request.process_seccomp_enabled,
        probe.as_ref(),
    )?;
    if effective.degraded {
        if let Some(detail) = &effective.degrade_detail {
            eprintln!("actrailctl launch degraded: {detail}");
        }
    }
    let payload_tls_seccomp_enabled = effective.payload_tls_seccomp_enabled;
    let seccomp_enabled = effective.use_seccomp;
    let payload_socket_enabled = effective.payload_socket_enabled;
    let process_seccomp_enabled = effective.process_seccomp_enabled;
    let child_setup = if seccomp_enabled {
        ChildSetup::Seccomp(seccomp_setup(
            &request,
            payload_tls_seccomp_enabled,
            payload_socket_enabled,
            process_seccomp_enabled,
        )?)
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
    let child_ref = process_ref(child.pid())?;

    let reply = match client.send(ControlCommand::TrackAdd(TrackAddCommand {
        request_id,
        root: child_ref.clone(),
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

    if let Err(error) = register_seccomp_listener_if_needed(
        client,
        request_id,
        trace_id,
        seccomp_enabled,
        &child,
        child_ref,
    ) {
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
    payload_socket_enabled: bool,
    process_seccomp_enabled: bool,
) -> Result<SeccompSetup, String> {
    let payload_tls_seccomp_syscalls = if payload_tls_seccomp_enabled {
        request.payload_tls_seccomp_syscalls.clone()
    } else {
        Vec::new()
    };
    let payload_socket_seccomp_syscalls = if payload_socket_enabled {
        request.payload_socket_seccomp_syscalls.clone()
    } else {
        Vec::new()
    };
    let process_seccomp_syscalls = if process_seccomp_enabled {
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

fn run_platform_probe_from_launch(request: &LaunchRequest) -> LaunchPlatformReport {
    LaunchPlatformReport {
        control_socket: linux_platform::capability_probe::CapabilityStatus::ok(
            "control_socket",
            "not checked for launch",
        ),
        tls_sync_socket: if tls_sync_enabled_request(request) {
            probe_unix_socket(&request.payload_tls_config.sync_event_socket_path)
        } else {
            linux_platform::capability_probe::CapabilityStatus::ok(
                "tls_sync_socket",
                "disabled by launch request",
            )
        },
        no_new_privs: probe_no_new_privs(),
        seccomp_launch: probe_seccomp_launch_capability(
            request.seccomp_notify_reserved_listener_fd,
        ),
        tls_sync_runtime_library: if tls_sync_enabled_request(request) {
            probe_tls_sync_runtime_library(&request.payload_tls_config)
        } else {
            linux_platform::capability_probe::CapabilityStatus::ok(
                "tls_sync_runtime_library",
                "disabled by launch request",
            )
        },
        daemon: None,
    }
}

fn tls_sync_enabled_request(request: &LaunchRequest) -> bool {
    request.payload_tls_config.enabled && request.payload_tls_config.capture_backend.is_sync()
}

fn register_seccomp_listener_if_needed(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    trace_id: TraceId,
    seccomp_enabled: bool,
    child: &ControlledChild,
    child_ref: control_contract::command::ProcessRef,
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
        child_ref,
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

//! Launch command orchestration.

#[path = "launch/controlled.rs"]
pub(crate) mod controlled;
#[path = "launch/java_agent.rs"]
mod java_agent;
#[path = "launch/permission_policy.rs"]
pub(crate) mod permission_policy;
#[path = "launch/seccomp.rs"]
pub(crate) mod seccomp;
#[path = "launch/suppress.rs"]
mod suppress;
#[path = "launch/sync.rs"]
mod sync;
#[path = "launch/timing.rs"]
mod timing;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use config_core::capture_profile::CaptureProfile;
use config_core::daemon::{
    NetworkControlSeccompSyscall, PayloadSocketSeccompSyscall, PayloadTlsConfig,
    PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use control_contract::command::{
    ControlCommand, ResolveLaunchPermissionsCommand, TrackAddCommand, TrackRemoveCommand,
};
use control_contract::reply::{ControlError, ControlReply};
use control_contract::selector::TraceSelector;
use model_core::ids::{RequestId, TraceId, TraceName};
use model_core::process::SuppressedFdPurpose;

use crate::output::format_reply;
use crate::process_ref::process_ref;
use crate::transport::ControlClientPort;
use controlled::{ChildSetup, ControlledChild};
use permission_policy::{
    DeploymentPermissionPolicy, LaunchSeccompRequirements, contract_permission_mode,
    permission_decision_from_reply,
};
use seccomp::{SeccompSetup, register_listener};
use suppress::InheritableSuppressedFd;
use sync::{SyncLaunch, sync_launch, sync_launch_envs};
use timing::LaunchTiming;

use crate::platform_probe::{
    LaunchPlatformReport, print_permission_decision, probe_seccomp_notify_capability,
    probe_tls_sync_runtime_library,
};
use linux_platform::capability_probe::{probe_no_new_privs, probe_unix_socket};

pub(crate) struct LaunchRequest {
    pub control_socket_path: PathBuf,
    pub display_name: TraceName,
    pub capture_profile: CaptureProfile,
    pub tags: BTreeSet<String>,
    pub payload_tls_config: PayloadTlsConfig,
    pub payload_tls_seccomp_syscalls: Vec<PayloadTlsSeccompSyscall>,
    pub payload_socket_seccomp_syscalls: Vec<PayloadSocketSeccompSyscall>,
    pub payload_socket_max_segment_bytes: u32,
    pub process_seccomp_syscalls: Vec<ProcessSeccompSyscall>,
    pub network_control_syscalls: Vec<NetworkControlSeccompSyscall>,
    pub seccomp_notify_reserved_listener_fd: u32,
    pub agent_invocation_commands: Vec<String>,
    pub supervision_poll_interval_ms: u64,
    pub ebpf_seccomp_policy: DeploymentPermissionPolicy,
    pub argv: Vec<String>,
}

pub(crate) fn run_launch(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    request: LaunchRequest,
) -> Result<i32, String> {
    let mut timing = LaunchTiming::from_env();
    timing.mark("start");
    let tls_sync_enabled =
        request.payload_tls_config.enabled && request.payload_tls_config.capture_backend.is_sync();
    timing.mark_detail(
        "classify_launch",
        format_args!("tls_sync_enabled={tls_sync_enabled}"),
    );
    let probe = run_platform_probe_from_launch(&request);
    timing.mark_detail(
        "platform_probe",
        format_args!(
            "seccomp_notify_available={} tls_sync_socket={} tls_sync_runtime_library={}",
            probe.seccomp_notify_available(),
            probe.tls_sync_socket.available,
            probe.tls_sync_runtime_library.available
        ),
    );
    let permission_reply = client
        .send(ControlCommand::ResolveLaunchPermissions(
            ResolveLaunchPermissionsCommand {
                request_id,
                profile_name: request.capture_profile.name.clone(),
                host_ebpf: contract_permission_mode(request.ebpf_seccomp_policy.host_ebpf),
                seccomp_notify: contract_permission_mode(
                    request.ebpf_seccomp_policy.seccomp_notify,
                ),
                seccomp_notify_available: probe.seccomp_notify_available(),
                seccomp_notify_detail: probe.seccomp_notify.detail.clone(),
            },
        ))
        .map_err(format_control_error)?;
    timing.mark("resolve_launch_permissions");
    let ControlReply::LaunchPermissions(permission_reply) = permission_reply else {
        return Err("resolve launch permissions returned unexpected reply".to_string());
    };
    let permissions = permission_decision_from_reply(&permission_reply);
    print_permission_decision(&permissions);
    let effective_seccomp = LaunchSeccompRequirements::new(
        permission_reply.payload_tls_seccomp,
        permission_reply.payload_socket_seccomp,
        permission_reply.process_seccomp,
        permission_reply.network_control_seccomp,
    );
    let seccomp_enabled = effective_seccomp.requires_seccomp_notify();
    timing.mark_detail(
        "permission_decision",
        format_args!(
            "seccomp_enabled={seccomp_enabled} payload_tls_seccomp={} payload_socket_seccomp={} process_seccomp={} network_control_seccomp={}",
            effective_seccomp.payload_tls,
            effective_seccomp.payload_socket,
            effective_seccomp.process_seccomp,
            effective_seccomp.network_control
        ),
    );
    let launch_supervision_poll_interval = if seccomp_enabled {
        Some(launch_supervision_poll_interval(&request)?)
    } else {
        None
    };
    let child_setup = if seccomp_enabled {
        ChildSetup::Seccomp(seccomp_setup(
            &request,
            effective_seccomp.payload_tls,
            effective_seccomp.payload_socket,
            effective_seccomp.process_seccomp,
            effective_seccomp.network_control,
        )?)
    } else {
        ChildSetup::Plain
    };
    timing.mark("build_child_setup");

    let raw_argv = request.argv;
    let sync_launch = if tls_sync_enabled {
        Some(sync_launch(
            client,
            request_id,
            raw_argv.clone(),
            &request.payload_tls_config,
            &request.agent_invocation_commands,
            &mut timing,
        )?)
    } else {
        None
    };
    timing.mark_detail("sync_launch", format_args!("enabled={tls_sync_enabled}"));
    let command = sync_launch
        .as_ref()
        .map(|launch| launch.command.clone())
        .unwrap_or_else(|| raw_argv.into_iter().map(OsString::from).collect());
    timing.mark_detail(
        "resolve_launch_command",
        format_args!("argc={}", command.len()),
    );
    let mut sync_event_fd = if tls_sync_enabled {
        Some(InheritableSuppressedFd::connect_unix_socket(
            &request.payload_tls_config.sync_event_socket_path,
            SuppressedFdPurpose::TlsSyncEvent,
        )?)
    } else {
        None
    };
    timing.mark_detail(
        "connect_sync_event_socket",
        format_args!("enabled={tls_sync_enabled}"),
    );
    let mut child = ControlledChild::spawn(command, child_setup)?;
    timing.mark_detail("spawn_stopped_child", format_args!("pid={}", child.pid()));
    let child_ref = process_ref(child.pid())?;
    timing.mark("resolve_child_process_ref");

    let track_add_request_id = next_request_id(request_id)?;
    let reply = match client.send(ControlCommand::TrackAdd(TrackAddCommand {
        request_id: track_add_request_id,
        root: child_ref.clone(),
        display_name: request.display_name,
        profile_name: permission_reply.selected_profile_name,
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
    timing.mark("track_add");
    let trace_id = match track_added_trace_id(&reply) {
        Ok(trace_id) => trace_id,
        Err(error) => {
            child.terminate();
            return Err(error);
        }
    };
    println!("{}", format_reply(&reply));
    timing.mark_detail("track_added", format_args!("trace_id={trace_id}"));

    if let Err(error) = register_seccomp_listener_if_needed(
        client,
        track_add_request_id,
        trace_id,
        seccomp_enabled,
        &child,
        child_ref,
    ) {
        child.terminate();
        remove_launch_root_best_effort(
            client,
            launch_remove_request_id(track_add_request_id, seccomp_enabled)?,
            trace_id,
        );
        return Err(error);
    }
    timing.mark_detail(
        "register_seccomp_listener",
        format_args!("enabled={seccomp_enabled}"),
    );
    if seccomp_enabled {
        child.close_listener_fd();
    }
    timing.mark_detail(
        "close_listener_fd",
        format_args!("enabled={seccomp_enabled}"),
    );
    let envs = match launch_envs(
        trace_id,
        &request.payload_tls_config,
        request.payload_socket_max_segment_bytes,
        sync_launch.as_ref(),
        sync_event_fd.as_ref(),
        &mut timing,
    ) {
        Ok(envs) => envs,
        Err(error) => {
            child.terminate();
            remove_launch_root_best_effort(
                client,
                launch_remove_request_id(track_add_request_id, seccomp_enabled)?,
                trace_id,
            );
            return Err(error);
        }
    };
    timing.mark_detail("launch_envs", format_args!("count={}", envs.len()));
    drop(sync_event_fd.take());
    timing.mark_detail(
        "drop_sync_event_fd_parent_copy",
        format_args!("enabled={tls_sync_enabled}"),
    );
    if let Err(error) = child.continue_with_envs(envs) {
        child.terminate();
        remove_launch_root_best_effort(
            client,
            launch_remove_request_id(track_add_request_id, seccomp_enabled)?,
            trace_id,
        );
        return Err(error);
    }
    timing.mark("continue_child");
    let child_result = if seccomp_enabled {
        child.wait_with_monitor(
            || control_socket_available(&request.control_socket_path),
            launch_supervision_poll_interval
                .expect("seccomp launch computed supervision poll interval"),
        )
    } else {
        child.wait()
    };
    timing.mark("child_wait_finished");
    let remove_result = remove_launch_root(
        client,
        launch_remove_request_id(track_add_request_id, seccomp_enabled)?,
        trace_id,
    );
    timing.mark("remove_launch_root");
    match (child_result, remove_result) {
        (Ok(status), Ok(())) => Ok(status),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Ok(())) => Err(error),
        (Err(child_error), Err(remove_error)) => {
            Err(format!("{child_error}; cleanup failed: {remove_error}"))
        }
    }
}

fn seccomp_setup(
    request: &LaunchRequest,
    payload_tls_seccomp_enabled: bool,
    payload_socket_enabled: bool,
    process_seccomp_enabled: bool,
    network_control_enabled: bool,
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
    let network_control_syscalls = if network_control_enabled {
        request.network_control_syscalls.clone()
    } else {
        Vec::new()
    };
    SeccompSetup::new(
        payload_tls_seccomp_syscalls,
        payload_socket_seccomp_syscalls,
        request.payload_socket_max_segment_bytes,
        process_seccomp_syscalls,
        network_control_syscalls,
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
        seccomp_notify: probe_seccomp_notify_capability(
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
    timing: &mut LaunchTiming,
) -> Result<Vec<(OsString, OsString)>, String> {
    match sync_launch {
        Some(sync_launch) => sync_launch_envs(
            trace_id,
            payload_tls_config,
            payload_socket_max_segment_bytes,
            sync_launch,
            sync_event_fd,
            timing,
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

fn control_socket_available(path: &Path) -> Result<bool, String> {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(format!(
            "check daemon control socket {}: {error}",
            path.display()
        )),
    }
}

fn launch_supervision_poll_interval(request: &LaunchRequest) -> Result<Duration, String> {
    if request.supervision_poll_interval_ms == 0 {
        return Err("supervision.poll_interval_ms must be greater than 0".to_string());
    }
    Ok(Duration::from_millis(request.supervision_poll_interval_ms))
}

fn format_control_error(error: ControlError) -> String {
    format!("control command failed: {}: {}", error.code, error.message)
}

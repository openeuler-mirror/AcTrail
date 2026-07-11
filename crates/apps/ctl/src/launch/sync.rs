//! Launch-time TLS sync runtime injection.

use std::ffi::OsString;
use std::path::PathBuf;

use config_core::daemon::{
    DisabledOrPath, PayloadTlsConfig, PayloadTlsLibraryPath, PayloadTlsSyncRuntimeLibraryPath,
};
use model_core::ids::RequestId;
use model_core::ids::TraceId;
use tls_payload_sync::{
    EventFilter, LibcFamily, RedactionMode, RuntimeEnvConfig, RuntimeFlowControlConfig,
    RuntimeLibraryPath, RuntimeLibrarySet, RuntimePlanDescriptor, audit_bind_now_env,
    audit_env_value_for_libraries, launch_command_for_plan_descriptor,
    preload_env_value_for_libraries, resolve_program_path, resolve_target_runtime,
    runtime_env_for_plan_descriptors, runtime_library_envs, runtime_library_set,
};

use super::java_agent::{java_agent_env_required, maybe_append_java_agent_env};
use super::suppress::InheritableSuppressedFd;
use super::timing::LaunchTiming;
use crate::tls_plan::{QueriedLaunchTlsPlan, query_launch_tls_plan};
use crate::transport::ControlClientPort;

pub(super) struct SyncLaunch {
    pub(super) command: Vec<OsString>,
    plans: Vec<RuntimePlanDescriptor>,
    runtime_libraries: RuntimeLibrarySet,
    initial_runtime_family: LibcFamily,
    preload_libraries: Vec<PathBuf>,
    audit_libraries: Vec<PathBuf>,
    java_agent_env_required: bool,
}

pub(super) fn sync_launch(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    argv: Vec<String>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
    timing: &mut LaunchTiming,
) -> Result<SyncLaunch, String> {
    if argv.is_empty() {
        return Err("launch requires a command after --".to_string());
    }
    validate_resolver_inputs(config)?;
    timing.mark("sync.validate_resolver_inputs");
    let raw_command = argv.into_iter().map(OsString::from).collect::<Vec<_>>();
    let (command, launch_plan) = match resolve_daemon_plan(client, request_id, &raw_command, config)
    {
        Ok(plan) => {
            let command = launch_command_for_plan_descriptor(&raw_command, &plan.descriptor)
                .map_err(|error| error.to_string())?;
            timing.mark_detail(
                "sync.resolve_launch_plan",
                format_args!(
                    "result=ok provider={} source={} binary={} cache={} daemon_elapsed_us={}",
                    plan.descriptor.provider,
                    plan.source,
                    plan.descriptor.binary.display(),
                    if plan.cache_hit { "hit" } else { "miss" },
                    plan.resolve_elapsed_micros
                ),
            );
            (command, Some(plan))
        }
        Err(error) => {
            timing.mark_detail(
                "sync.resolve_launch_plan",
                format_args!("result=miss error={error}"),
            );
            (raw_command, None)
        }
    };
    let runtime_libraries = runtime_libraries(config)?;
    timing.mark_detail(
        "sync.runtime_libraries",
        format_args!(
            "glibc={} musl={}",
            runtime_libraries.glibc.display(),
            runtime_libraries
                .musl
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
    );
    let initial_runtime_family = initial_runtime_family(&command)?;
    timing.mark_detail(
        "sync.initial_runtime_family",
        format_args!("family={}", initial_runtime_family.as_str()),
    );
    let preload_libraries = sync_preload_libraries(&runtime_libraries, initial_runtime_family)?;
    timing.mark_detail(
        "sync.preload_libraries",
        format_args!("count={}", preload_libraries.len()),
    );
    let audit_libraries = if initial_runtime_family == LibcFamily::Glibc {
        launch_plan
            .as_ref()
            .map(|plan| audit_libraries_for_plan_source(&preload_libraries, &plan.source))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    timing.mark_detail(
        "sync.audit_libraries",
        format_args!("count={}", audit_libraries.len()),
    );
    let plans = bundle_plans(
        client,
        request_id,
        launch_plan.as_ref(),
        config,
        agent_commands,
    );
    timing.mark_detail(
        "sync.bundle_plans",
        format_args!(
            "count={} agent_command_count={}",
            plans.len(),
            agent_commands.len()
        ),
    );
    let java_agent_env_required = java_agent_env_required(config);
    timing.mark_detail(
        "sync.java_agent_env_required",
        format_args!("enabled={java_agent_env_required}"),
    );
    Ok(SyncLaunch {
        command,
        plans,
        runtime_libraries,
        initial_runtime_family,
        preload_libraries,
        audit_libraries,
        java_agent_env_required,
    })
}

pub(super) fn sync_launch_envs(
    trace_id: TraceId,
    config: &PayloadTlsConfig,
    socket_max_segment_bytes: u32,
    launch: &SyncLaunch,
    sync_event_fd: Option<&InheritableSuppressedFd>,
    timing: &mut LaunchTiming,
) -> Result<Vec<(OsString, OsString)>, String> {
    let sync_event_fd = sync_event_fd
        .ok_or_else(|| "TLS sync launch requires an inherited event fd".to_string())?;
    timing.mark("sync_env.require_event_fd");
    let mut envs = runtime_env_for_plan_descriptors(
        &RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: usize::try_from(config.max_operation_bytes)
                .map_err(|error| format!("payload_tls_max_operation_bytes overflow: {error}"))?,
            flow_control: RuntimeFlowControlConfig {
                enabled: config.sync_flow_control_enabled,
                sniff_bytes: usize::try_from(config.sync_flow_sniff_bytes).map_err(|error| {
                    format!("payload_tls_sync_flow_sniff_bytes overflow: {error}")
                })?,
                max_header_bytes: usize::try_from(config.sync_flow_max_header_bytes).map_err(
                    |error| format!("payload_tls_sync_flow_max_header_bytes overflow: {error}"),
                )?,
                large_transfer_bytes: config.sync_flow_large_transfer_bytes,
                unknown_stream_bytes: config.sync_flow_unknown_stream_bytes,
                h2_data_probe_bytes: config.sync_flow_h2_data_probe_bytes,
            },
            redaction: RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: Some(trace_id.get()),
            event_socket_path: Some(config.sync_event_socket_path.clone()),
            event_fd: Some(sync_event_fd.raw_fd()),
            event_write_buffer_bytes: Some(
                usize::try_from(socket_max_segment_bytes).map_err(|error| {
                    format!("payload_socket_max_segment_bytes overflow: {error}")
                })?,
            ),
        },
        &launch.plans,
    )
    .map_err(|error| error.to_string())?;
    timing.mark_detail(
        "sync_env.runtime_env_for_plans",
        format_args!("plan_count={} env_count={}", launch.plans.len(), envs.len()),
    );
    envs.extend(runtime_library_envs(&launch.runtime_libraries));
    timing.mark_detail(
        "sync_env.runtime_library_envs",
        format_args!("env_count={}", envs.len()),
    );
    envs.push((
        OsString::from("LD_PRELOAD"),
        preload_env_value_for_libraries(&launch.preload_libraries)
            .map_err(|error| error.to_string())?,
    ));
    timing.mark_detail(
        "sync_env.ld_preload",
        format_args!("env_count={}", envs.len()),
    );
    let glibc_preload = vec![launch.runtime_libraries.glibc.clone()];
    if launch.initial_runtime_family == LibcFamily::Glibc {
        if let Some(env) =
            tls_payload_sync::runtime_dependency_library_path_env(&launch.preload_libraries)
                .map_err(|error| error.to_string())?
        {
            envs.push(env);
        }
        if let Some(env) =
            tls_payload_sync::runtime_dependency_library_path_prefix_env(&launch.preload_libraries)
                .map_err(|error| error.to_string())?
        {
            envs.push(env);
        }
    }
    timing.mark_detail(
        "sync_env.runtime_dependency_paths",
        format_args!("env_count={}", envs.len()),
    );
    if let Some(env) =
        tls_payload_sync::runtime_dependency_library_path_prefix_glibc_env(&glibc_preload)
            .map_err(|error| error.to_string())?
    {
        envs.push(env);
    }
    timing.mark_detail(
        "sync_env.runtime_dependency_paths_glibc",
        format_args!("env_count={}", envs.len()),
    );
    if !launch.audit_libraries.is_empty() {
        envs.push((
            OsString::from("LD_AUDIT"),
            audit_env_value_for_libraries(&launch.audit_libraries)
                .map_err(|error| error.to_string())?,
        ));
        if let Some(env) = audit_bind_now_env(&launch.audit_libraries) {
            envs.push(env);
        }
    }
    timing.mark_detail("sync_env.audit", format_args!("env_count={}", envs.len()));
    maybe_append_java_agent_env(launch.java_agent_env_required, &mut envs)?;
    timing.mark_detail(
        "sync_env.java_agent",
        format_args!("env_count={}", envs.len()),
    );
    Ok(envs)
}

fn bundle_plans(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    launch_plan: Option<&QueriedLaunchTlsPlan>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Vec<RuntimePlanDescriptor> {
    let mut plans = launch_plan
        .map(|plan| plan.descriptor.clone())
        .into_iter()
        .collect::<Vec<_>>();
    for command in agent_commands {
        let candidate = vec![OsString::from(command)];
        let Ok(plan) = resolve_daemon_plan(client, request_id, &candidate, config) else {
            continue;
        };
        if contains_plan(&plans, &plan.descriptor) {
            continue;
        }
        plans.push(plan.descriptor);
    }
    plans
}

fn resolve_daemon_plan(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    command: &[OsString],
    config: &PayloadTlsConfig,
) -> Result<QueriedLaunchTlsPlan, String> {
    match try_resolve_daemon_plan(client, request_id, command) {
        Ok(plan) => Ok(plan),
        Err(primary_error) => match &config.binary_path {
            DisabledOrPath::Path(path) => {
                query_launch_tls_plan(client, request_id, path)
                    .map_err(|fallback_error| {
                        format!(
                            "launch command probe failed ({primary_error}); payload_tls_binary_path fallback failed ({fallback_error})"
                        )
                    })
                    .and_then(|plan| {
                        plan.ok_or_else(|| {
                            format!(
                                "launch command probe failed ({primary_error}); payload_tls_binary_path fallback did not return a plan"
                            )
                        })
                    })
            }
            DisabledOrPath::Disabled => Err(primary_error),
        },
    }
}

fn try_resolve_daemon_plan(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    command: &[OsString],
) -> Result<QueriedLaunchTlsPlan, String> {
    let binary = resolve_command_binary(command)?;
    query_launch_tls_plan(client, request_id, &binary)
        .and_then(|plan| plan.ok_or_else(|| "daemon returned no TLS launch plan".to_string()))
}

fn resolve_command_binary(command: &[OsString]) -> Result<PathBuf, String> {
    let Some(program) = command.first() else {
        return Err("launch requires a command after --".to_string());
    };
    let path = std::env::var_os("PATH");
    resolve_program_path(program, path.as_ref().map(|value| value.as_os_str()))
        .map_err(|error| error.to_string())
}

fn contains_plan(plans: &[RuntimePlanDescriptor], candidate: &RuntimePlanDescriptor) -> bool {
    let candidate_path = canonical_path(&candidate.binary);
    plans
        .iter()
        .any(|plan| canonical_path(&plan.binary) == candidate_path)
}

fn sync_preload_libraries(
    runtime_libraries: &RuntimeLibrarySet,
    family: LibcFamily,
) -> Result<Vec<PathBuf>, String> {
    Ok(vec![
        runtime_libraries
            .library_for(family)
            .map_err(|error| error.to_string())?,
    ])
}

fn audit_libraries_for_plan_source(runtime_libraries: &[PathBuf], source: &str) -> Vec<PathBuf> {
    if source == "shared-library" {
        runtime_libraries.to_vec()
    } else {
        Vec::new()
    }
}

fn canonical_path(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn runtime_libraries(config: &PayloadTlsConfig) -> Result<RuntimeLibrarySet, String> {
    runtime_library_set(&match &config.sync_runtime_library_path {
        PayloadTlsSyncRuntimeLibraryPath::Auto => RuntimeLibraryPath::Auto,
        PayloadTlsSyncRuntimeLibraryPath::Path(path) => RuntimeLibraryPath::Path(path.clone()),
    })
    .map_err(|error| error.to_string())
}

fn initial_runtime_family(command: &[OsString]) -> Result<LibcFamily, String> {
    let Some(program) = command.first() else {
        return Err("launch requires a command after --".to_string());
    };
    let path = std::env::var_os("PATH");
    resolve_target_runtime(program, path.as_ref().map(|value| value.as_os_str()))
        .map(|target| target.libc)
        .map_err(|error| format!("TLS sync runtime target detection failed: {error}"))
}

fn library_candidates(config: &PayloadTlsConfig) -> Vec<PathBuf> {
    match &config.library_path {
        PayloadTlsLibraryPath::Auto => Vec::new(),
        PayloadTlsLibraryPath::Path(path) => vec![path.clone()],
    }
}

fn validate_resolver_inputs(config: &PayloadTlsConfig) -> Result<(), String> {
    let _ = match_limit(config)?;
    for path in library_candidates(config) {
        if !path.is_file() {
            return Err(format!(
                "payload_tls_library_path is not a file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn match_limit(config: &PayloadTlsConfig) -> Result<usize, String> {
    usize::try_from(config.sync_match_limit)
        .map_err(|error| format!("payload_tls_sync_match_limit overflow: {error}"))
}

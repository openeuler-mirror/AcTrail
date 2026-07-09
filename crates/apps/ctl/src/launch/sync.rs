//! Launch-time TLS sync runtime injection.

use std::ffi::OsString;
use std::path::PathBuf;

use config_core::daemon::{
    DisabledOrPath, PayloadTlsConfig, PayloadTlsLibraryPath, PayloadTlsSyncRuntimeLibraryPath,
};
use model_core::ids::TraceId;
use tls_payload_sync::{
    EventFilter, LibcFamily, RedactionMode, RuntimeEnvConfig, RuntimeFlowControlConfig,
    RuntimeLibraryPath, RuntimeLibrarySet, audit_bind_now_env, audit_env_value_for_libraries,
    audit_libraries_for_plans, launch_command_for_plan, preload_env_value_for_libraries,
    resolve_target_runtime, runtime_env_for_plans, runtime_library_envs, runtime_library_set,
    validate_native_backend_plan,
};
use tls_probe_point_finder::ProbePointPlan;
use tls_probe_point_finder::fast::{ArchFilter, FastProbeRequest, ProviderFilter, SourceFilter};

use super::java_agent::{java_agent_env_required, maybe_append_java_agent_env};
use super::suppress::InheritableSuppressedFd;

pub(super) struct SyncLaunch {
    pub(super) command: Vec<OsString>,
    plans: Vec<ProbePointPlan>,
    runtime_libraries: RuntimeLibrarySet,
    initial_runtime_family: LibcFamily,
    preload_libraries: Vec<PathBuf>,
    audit_libraries: Vec<PathBuf>,
    java_agent_env_required: bool,
}

pub(super) fn sync_launch(
    argv: Vec<String>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Result<SyncLaunch, String> {
    if argv.is_empty() {
        return Err("launch requires a command after --".to_string());
    }
    validate_resolver_inputs(config)?;
    let raw_command = argv.into_iter().map(OsString::from).collect::<Vec<_>>();
    let (command, launch_plan) = match resolve_native_plan(&raw_command, config) {
        Ok(plan) => {
            let command =
                launch_command_for_plan(&raw_command, &plan).map_err(|error| error.to_string())?;
            (command, Some(plan))
        }
        Err(_) => (raw_command, None),
    };
    let runtime_libraries = runtime_libraries(config)?;
    let initial_runtime_family = initial_runtime_family(&command)?;
    let preload_libraries = sync_preload_libraries(&runtime_libraries, initial_runtime_family)?;
    let audit_libraries = if initial_runtime_family == LibcFamily::Glibc {
        launch_plan
            .as_ref()
            .map(|plan| audit_libraries_for_plans(&preload_libraries, std::slice::from_ref(plan)))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let plans = bundle_plans(launch_plan, config, agent_commands);
    let java_agent_env_required = java_agent_env_required(config);
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
) -> Result<Vec<(OsString, OsString)>, String> {
    let sync_event_fd = sync_event_fd
        .ok_or_else(|| "TLS sync launch requires an inherited event fd".to_string())?;
    let mut envs = runtime_env_for_plans(
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
    envs.extend(runtime_library_envs(&launch.runtime_libraries));
    envs.push((
        OsString::from("LD_PRELOAD"),
        preload_env_value_for_libraries(&launch.preload_libraries)
            .map_err(|error| error.to_string())?,
    ));
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
    if let Some(env) =
        tls_payload_sync::runtime_dependency_library_path_prefix_glibc_env(&glibc_preload)
            .map_err(|error| error.to_string())?
    {
        envs.push(env);
    }
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
    maybe_append_java_agent_env(launch.java_agent_env_required, &mut envs)?;
    Ok(envs)
}

fn bundle_plans(
    launch_plan: Option<ProbePointPlan>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Vec<ProbePointPlan> {
    let mut plans = launch_plan.into_iter().collect::<Vec<_>>();
    for command in agent_commands {
        let candidate = vec![OsString::from(command)];
        let Ok(plan) = resolve_native_plan(&candidate, config) else {
            continue;
        };
        if contains_plan(&plans, &plan) {
            continue;
        }
        plans.push(plan);
    }
    plans
}

fn resolve_native_plan(
    command: &[OsString],
    config: &PayloadTlsConfig,
) -> Result<ProbePointPlan, String> {
    let plan = resolve_plan(command, config)?;
    validate_native_backend_plan(&plan).map_err(|error| error.to_string())?;
    Ok(plan)
}

fn contains_plan(plans: &[ProbePointPlan], candidate: &ProbePointPlan) -> bool {
    let candidate_path = canonical_path(&candidate.binary.path);
    plans
        .iter()
        .any(|plan| canonical_path(&plan.binary.path) == candidate_path)
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

fn canonical_path(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_plan(command: &[OsString], config: &PayloadTlsConfig) -> Result<ProbePointPlan, String> {
    match try_resolve_plan(command, config) {
        Ok(plan) => Ok(plan),
        Err(primary_error) => match &config.binary_path {
            DisabledOrPath::Path(path) => {
                let fallback_command = vec![OsString::from(path.as_os_str())];
                try_resolve_plan(&fallback_command, config).map_err(|fallback_error| {
                    format!(
                        "launch command probe failed ({primary_error}); payload_tls_binary_path fallback failed ({fallback_error})"
                    )
                })
            }
            DisabledOrPath::Disabled => Err(primary_error),
        },
    }
}

fn try_resolve_plan(
    command: &[OsString],
    config: &PayloadTlsConfig,
) -> Result<ProbePointPlan, String> {
    let Some(program) = command.first() else {
        return Err("launch requires a command after --".to_string());
    };
    tls_probe_point_finder::fast::resolve(FastProbeRequest {
        binary: program.into(),
        arch: ArchFilter::Auto,
        provider: ProviderFilter::Auto,
        source: SourceFilter::Auto,
        match_limit: match_limit(config)?,
        libraries: library_candidates(config),
        library_search_dirs: Vec::new(),
    })
    .map_err(|error| error.to_string())
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

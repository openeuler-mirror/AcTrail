//! Launch-time TLS sync runtime injection.

use std::ffi::OsString;
use std::path::PathBuf;

use config_core::daemon::{
    PayloadTlsConfig, PayloadTlsLibraryPath, PayloadTlsSyncRuntimeLibraryPath,
};
use model_core::ids::TraceId;
use tls_payload_sync::{
    EventFilter, RedactionMode, RuntimeEnvConfig, RuntimeLibraryPath, launch_command_for_plan,
    preload_env_value, run_with_preload, runtime_env_for_plans, runtime_library_path,
    validate_native_backend_plan,
};
use tls_probe_point_finder::fast::{ArchFilter, FastProbeRequest, ProviderFilter, SourceFilter};

pub(super) struct SyncLaunch {
    pub(super) command: Vec<OsString>,
    pub(super) envs: Vec<(OsString, OsString)>,
    plan: tls_probe_point_finder::ProbePointPlan,
    library: PathBuf,
}

pub(super) fn run_child_sync_tls(
    trace_id: TraceId,
    argv: Vec<String>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Result<i32, String> {
    let launch = sync_launch(trace_id, argv, config, agent_commands)?;
    let status = run_with_preload(&launch.command, &launch.plan, &launch.library, launch.envs)
        .map_err(|error| error.to_string())?;
    status
        .code()
        .ok_or_else(|| "launch child terminated without an exit code".to_string())
}

pub(super) fn sync_launch(
    trace_id: TraceId,
    argv: Vec<String>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Result<SyncLaunch, String> {
    if argv.is_empty() {
        return Err("launch requires a command after --".to_string());
    }
    let raw_command = argv.into_iter().map(OsString::from).collect::<Vec<_>>();
    let plan = resolve_native_plan(&raw_command, config)?;
    let command =
        launch_command_for_plan(&raw_command, &plan).map_err(|error| error.to_string())?;
    let library = runtime_library(config)?;
    let plans = bundle_plans(plan.clone(), config, agent_commands);
    let mut envs = runtime_env_for_plans(
        &RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: usize::try_from(config.max_operation_bytes)
                .map_err(|error| format!("payload_tls_max_operation_bytes overflow: {error}"))?,
            redaction: RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: Some(trace_id.get()),
            event_socket_path: Some(config.sync_event_socket_path.clone()),
        },
        &plans,
    )
    .map_err(|error| error.to_string())?;
    envs.push((
        OsString::from("LD_PRELOAD"),
        preload_env_value(&library).map_err(|error| error.to_string())?,
    ));
    Ok(SyncLaunch {
        command,
        envs,
        plan,
        library,
    })
}

fn bundle_plans(
    launch_plan: tls_probe_point_finder::ProbePointPlan,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Vec<tls_probe_point_finder::ProbePointPlan> {
    let mut plans = vec![launch_plan];
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
) -> Result<tls_probe_point_finder::ProbePointPlan, String> {
    let plan = resolve_plan(command, config)?;
    validate_native_backend_plan(&plan).map_err(|error| error.to_string())?;
    Ok(plan)
}

fn contains_plan(
    plans: &[tls_probe_point_finder::ProbePointPlan],
    candidate: &tls_probe_point_finder::ProbePointPlan,
) -> bool {
    let candidate_path = canonical_path(&candidate.binary.path);
    plans
        .iter()
        .any(|plan| canonical_path(&plan.binary.path) == candidate_path)
}

fn canonical_path(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_plan(
    command: &[OsString],
    config: &PayloadTlsConfig,
) -> Result<tls_probe_point_finder::ProbePointPlan, String> {
    let Some(program) = command.first() else {
        return Err("launch requires a command after --".to_string());
    };
    tls_probe_point_finder::fast::resolve(FastProbeRequest {
        binary: program.into(),
        arch: ArchFilter::Auto,
        provider: ProviderFilter::Auto,
        source: SourceFilter::Auto,
        match_limit: usize::try_from(config.sync_match_limit)
            .map_err(|error| format!("payload_tls_sync_match_limit overflow: {error}"))?,
        libraries: library_candidates(config),
        library_search_dirs: Vec::new(),
    })
    .map_err(|error| error.to_string())
}

fn runtime_library(config: &PayloadTlsConfig) -> Result<PathBuf, String> {
    runtime_library_path(&match &config.sync_runtime_library_path {
        PayloadTlsSyncRuntimeLibraryPath::Auto => RuntimeLibraryPath::Auto,
        PayloadTlsSyncRuntimeLibraryPath::Path(path) => RuntimeLibraryPath::Path(path.clone()),
    })
    .map_err(|error| error.to_string())
}

fn library_candidates(config: &PayloadTlsConfig) -> Vec<PathBuf> {
    match &config.library_path {
        PayloadTlsLibraryPath::Auto => Vec::new(),
        PayloadTlsLibraryPath::Path(path) => vec![path.clone()],
    }
}

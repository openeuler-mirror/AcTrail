//! Launch-time TLS sync runtime injection.

use std::ffi::OsString;
use std::path::PathBuf;

use config_core::daemon::{
    PayloadTlsConfig, PayloadTlsLibraryPath, PayloadTlsSyncRuntimeLibraryPath,
};
use model_core::ids::TraceId;
use tls_payload_sync::{
    EventFilter, RedactionMode, RuntimeEnvConfig, RuntimeLibraryPath, launch_command_for_plan,
    preload_env_value_for_libraries, runtime_env_for_plans, runtime_library_path,
    validate_native_backend_plan,
};
use tls_probe_point_finder::ProbePointPlan;
use tls_probe_point_finder::fast::{ArchFilter, FastProbeRequest, ProviderFilter, SourceFilter};

use super::java_agent::{java_agent_env_required, maybe_append_java_agent_env};

pub(super) struct SyncLaunch {
    pub(super) command: Vec<OsString>,
    plans: Vec<ProbePointPlan>,
    preload_libraries: Vec<PathBuf>,
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
    let runtime_library = runtime_library(config)?;
    let plans = bundle_plans(launch_plan, config, agent_commands);
    let preload_libraries = sync_preload_libraries(&runtime_library);
    let java_agent_env_required = java_agent_env_required(config);
    Ok(SyncLaunch {
        command,
        plans,
        preload_libraries,
        java_agent_env_required,
    })
}

pub(super) fn sync_launch_envs(
    trace_id: TraceId,
    config: &PayloadTlsConfig,
    launch: &SyncLaunch,
) -> Result<Vec<(OsString, OsString)>, String> {
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
        &launch.plans,
    )
    .map_err(|error| error.to_string())?;
    envs.push((
        OsString::from("LD_PRELOAD"),
        preload_env_value_for_libraries(&launch.preload_libraries)
            .map_err(|error| error.to_string())?,
    ));
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

fn sync_preload_libraries(runtime_library: &PathBuf) -> Vec<PathBuf> {
    vec![runtime_library.clone()]
}

fn canonical_path(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_plan(command: &[OsString], config: &PayloadTlsConfig) -> Result<ProbePointPlan, String> {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::sync_preload_libraries;

    #[test]
    fn sync_preload_uses_only_runtime_carrier() {
        let runtime = PathBuf::from("/tmp/libactrail_tls_payload_probe_sync.so");

        let libraries = sync_preload_libraries(&runtime);

        assert_eq!(libraries, vec![runtime]);
    }
}

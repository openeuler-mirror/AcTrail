use super::super::super::java_agent::maybe_append_java_agent_env;
use super::super::super::timing::LaunchTiming;
use super::SyncLaunch;
use config_core::daemon::PayloadTlsConfig;
use model_core::ids::TraceId;
use std::ffi::OsString;
use tls_payload_sync::{
    ENV_EVENT_BATCH_BYTES, ENV_EVENT_SHUTDOWN_TIMEOUT_MS, ENV_EVENT_TIMEOUT_MS, EventFilter,
    LibcFamily, RedactionMode, RuntimeEnvConfig, RuntimeFlowControlConfig, audit_bind_now_env,
    audit_env_value_for_libraries, preload_env_value_for_libraries,
    runtime_env_for_plan_descriptors, runtime_library_envs,
};

pub(super) fn sync_launch_envs(
    trace_id: TraceId,
    config: &PayloadTlsConfig,
    launch: &SyncLaunch,
    timing: &mut LaunchTiming,
) -> Result<Vec<(OsString, OsString)>, String> {
    if !launch.sync_runtime_required {
        return Ok(Vec::new());
    }
    let mut envs = runtime_env_for_plan_descriptors(
        &RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: usize::try_from(config.max_operation_bytes)
                .map_err(|error| format!("payload_tls_max_operation_bytes overflow: {error}"))?,
            max_frame_bytes: usize::try_from(config.sync_max_frame_bytes)
                .map_err(|error| format!("payload_tls_sync_max_frame_bytes overflow: {error}"))?,
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
                max_streams: 4096,
            },
            redaction: RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: Some(trace_id.get()),
            event_socket_path: Some(config.sync_event_socket_path.clone()),
        },
        &launch.plans,
    )
    .map_err(|error| error.to_string())?;
    for (key, value) in [
        (
            ENV_EVENT_TIMEOUT_MS,
            config.sync_event_timeout_ms.to_string(),
        ),
        (
            ENV_EVENT_SHUTDOWN_TIMEOUT_MS,
            config.sync_event_shutdown_timeout_ms.to_string(),
        ),
        (
            ENV_EVENT_BATCH_BYTES,
            config.sync_event_batch_bytes.to_string(),
        ),
    ] {
        envs.push((OsString::from(key), OsString::from(value)));
    }
    timing.mark_detail(
        "sync_env.runtime_env_for_plans",
        format_args!("plan_count={} env_count={}", launch.plans.len(), envs.len()),
    );
    let runtime_libraries = launch
        .runtime_libraries
        .as_ref()
        .ok_or_else(|| "TLS sync launch has no runtime library set".to_string())?;
    let initial_runtime_family = launch
        .initial_runtime_family
        .ok_or_else(|| "TLS sync launch has no runtime family".to_string())?;
    envs.extend(runtime_library_envs(runtime_libraries));
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
    let glibc_preload = vec![runtime_libraries.glibc.clone()];
    if initial_runtime_family == LibcFamily::Glibc {
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

//! TLS sync injection preparation.

#[path = "tls_sync/env.rs"]
mod env;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use config_core::daemon::{
    DisabledOrPath, PayloadTlsConfig, PayloadTlsLibraryPath, PayloadTlsSyncRuntimeLibraryPath,
};
use control_contract::command::LaunchTlsProbePlan;
use model_core::ids::RequestId;
use model_core::process::SuppressedFdPurpose;
use tls_payload_sync::{
    LibcFamily, RuntimeLibraryPath, RuntimeLibrarySet, RuntimePlanDescriptor, TargetRuntime,
    launch_command_for_plan_descriptor, resolve_program_path, resolve_target_runtime,
    runtime_library_set, target_runtime_for_path,
};

use super::super::java_agent::java_agent_env_required;
use super::super::suppress::InheritableSuppressedFd;
use super::super::timing::LaunchTiming;
use crate::tls_plan::{QueriedLaunchTlsPlan, query_launch_tls_plan};
use crate::transport::ControlClientPort;

struct SyncLaunch {
    command: Vec<OsString>,
    plans: Vec<RuntimePlanDescriptor>,
    sync_runtime_required: bool,
    runtime_libraries: Option<RuntimeLibrarySet>,
    initial_runtime_family: Option<LibcFamily>,
    preload_libraries: Vec<PathBuf>,
    audit_libraries: Vec<PathBuf>,
    direct_probe_plans: Vec<RuntimePlanDescriptor>,
    java_agent_env_required: bool,
}

impl SyncLaunch {
    fn direct(command: Vec<OsString>, direct_probe_plans: Vec<RuntimePlanDescriptor>) -> Self {
        Self {
            command,
            plans: Vec::new(),
            sync_runtime_required: false,
            runtime_libraries: None,
            initial_runtime_family: None,
            preload_libraries: Vec::new(),
            audit_libraries: Vec::new(),
            direct_probe_plans,
            java_agent_env_required: false,
        }
    }
}

struct LaunchPlanSet {
    runtime: Vec<RuntimePlanDescriptor>,
    direct: Vec<RuntimePlanDescriptor>,
}

impl LaunchPlanSet {
    fn from_launch_plan(launch_plan: Option<&QueriedLaunchTlsPlan>) -> Result<Self, String> {
        let mut plans = Self {
            runtime: Vec::new(),
            direct: Vec::new(),
        };
        if let Some(launch_plan) = launch_plan {
            for descriptor in launch_plan.descriptors.iter().cloned() {
                plans.add_plan(descriptor)?;
            }
        }
        Ok(plans)
    }

    fn add_plan(&mut self, descriptor: RuntimePlanDescriptor) -> Result<(), String> {
        if self.contains(&descriptor) {
            return Ok(());
        }
        let runtime = target_runtime_for_path(&descriptor.target, None).map_err(|error| {
            format!(
                "resolve launch TLS target {}: {error}",
                descriptor.target.display()
            )
        })?;
        if runtime.is_static() {
            self.direct.push(descriptor);
        } else {
            self.runtime.push(descriptor);
        }
        Ok(())
    }

    fn contains(&self, candidate: &RuntimePlanDescriptor) -> bool {
        self.runtime
            .iter()
            .chain(&self.direct)
            .any(|plan| same_plan(plan, candidate))
    }
}

fn sync_launch(
    client: &mut (impl ControlClientPort + ?Sized),
    request_id: RequestId,
    command: Vec<OsString>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
    timing: &mut LaunchTiming,
) -> Result<SyncLaunch, String> {
    if command.is_empty() {
        return Err("launch requires a command after --".to_string());
    }
    validate_resolver_inputs(config)?;
    timing.mark("sync.validate_resolver_inputs");
    let raw_command = command;
    let (command, launch_plan) = match resolve_daemon_plan(client, request_id, &raw_command, config)
    {
        Ok(plan) => {
            let primary = plan
                .descriptors
                .first()
                .ok_or_else(|| "TLS plan resolution returned an empty plan set".to_string())?;
            let command = launch_command_for_plan_descriptor(&raw_command, primary)
                .map_err(|error| error.to_string())?;
            timing.mark_detail(
                "sync.resolve_launch_plan",
                format_args!(
                    "result=ok provider={} source={} binary={} cache={} daemon_elapsed_us={}",
                    primary.provider,
                    plan.sources
                        .first()
                        .map(String::as_str)
                        .unwrap_or("unknown"),
                    primary.binary.display(),
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
    let initial_runtime = initial_runtime(&command)?;
    if initial_runtime.is_static() {
        let direct_probe_plans = launch_plan
            .as_ref()
            .map(|plan| plan.descriptors.clone())
            .unwrap_or_default();
        if direct_probe_plans.is_empty() {
            return Err(format!(
                "static ELF {} requires a resolved native TLS probe plan",
                initial_runtime.path.display()
            ));
        }
        timing.mark_detail(
            "sync.initial_runtime",
            format_args!(
                "kind=static binary={} provider={}",
                initial_runtime.path.display(),
                direct_probe_plans
                    .iter()
                    .map(|plan| plan.provider.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
        return Ok(SyncLaunch::direct(command, direct_probe_plans));
    }
    let initial_runtime_family = initial_runtime
        .libc
        .ok_or_else(|| "dynamic TLS target has no libc family".to_string())?;
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
            .map(|plan| {
                audit_libraries_for_plan_source(
                    &preload_libraries,
                    plan.sources
                        .first()
                        .map(String::as_str)
                        .unwrap_or("unknown"),
                )
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    timing.mark_detail(
        "sync.audit_libraries",
        format_args!("count={}", audit_libraries.len()),
    );
    let plans = launch_plans(
        client,
        request_id,
        launch_plan.as_ref(),
        config,
        agent_commands,
    )?;
    timing.mark_detail(
        "sync.bundle_plans",
        format_args!(
            "runtime_count={} direct_count={} agent_command_count={}",
            plans.runtime.len(),
            plans.direct.len(),
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
        plans: plans.runtime,
        sync_runtime_required: true,
        runtime_libraries: Some(runtime_libraries),
        initial_runtime_family: Some(initial_runtime_family),
        preload_libraries,
        audit_libraries,
        direct_probe_plans: plans.direct,
        java_agent_env_required,
    })
}

fn launch_plans(
    client: &mut (impl ControlClientPort + ?Sized),
    request_id: RequestId,
    launch_plan: Option<&QueriedLaunchTlsPlan>,
    config: &PayloadTlsConfig,
    agent_commands: &[String],
) -> Result<LaunchPlanSet, String> {
    let mut plans = LaunchPlanSet::from_launch_plan(launch_plan)?;
    for command in agent_commands {
        let candidate = vec![OsString::from(command)];
        let binary = match resolve_command_binary(&candidate) {
            Ok(binary) => binary,
            Err(_) => continue,
        };
        let plan = resolve_daemon_plan_for_binary(client, request_id, &binary, config)?;
        for descriptor in plan.descriptors {
            plans.add_plan(descriptor)?;
        }
    }
    Ok(plans)
}

fn resolve_daemon_plan(
    client: &mut (impl ControlClientPort + ?Sized),
    request_id: RequestId,
    command: &[OsString],
    config: &PayloadTlsConfig,
) -> Result<QueriedLaunchTlsPlan, String> {
    let binary = resolve_command_binary(command)?;
    resolve_daemon_plan_for_binary(client, request_id, &binary, config)
}

fn resolve_daemon_plan_for_binary(
    client: &mut (impl ControlClientPort + ?Sized),
    request_id: RequestId,
    binary: &Path,
    config: &PayloadTlsConfig,
) -> Result<QueriedLaunchTlsPlan, String> {
    match query_launch_tls_plan(client, request_id, binary)
        .and_then(|plan| plan.ok_or_else(|| "daemon returned no TLS launch plan".to_string()))
    {
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

fn resolve_command_binary(command: &[OsString]) -> Result<PathBuf, String> {
    let Some(program) = command.first() else {
        return Err("launch requires a command after --".to_string());
    };
    let path = std::env::var_os("PATH");
    let binary =
        resolve_program_path(program, path.as_deref()).map_err(|error| error.to_string())?;
    tls_probe_point_finder::EntryElfResolver::resolve(&binary).map_err(|error| error.to_string())
}

fn same_plan(plan: &RuntimePlanDescriptor, candidate: &RuntimePlanDescriptor) -> bool {
    canonical_path(&plan.target) == canonical_path(&candidate.target)
        && canonical_path(&plan.binary) == canonical_path(&candidate.binary)
        && plan.target_identity == candidate.target_identity
        && plan.binary_identity == candidate.binary_identity
        && plan.provider == candidate.provider
        && plan.points == candidate.points
}

fn canonical_path(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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

fn runtime_libraries(config: &PayloadTlsConfig) -> Result<RuntimeLibrarySet, String> {
    runtime_library_set(&match &config.sync_runtime_library_path {
        PayloadTlsSyncRuntimeLibraryPath::Auto => RuntimeLibraryPath::Auto,
        PayloadTlsSyncRuntimeLibraryPath::Path(path) => RuntimeLibraryPath::Path(path.clone()),
    })
    .map_err(|error| error.to_string())
}

fn initial_runtime(command: &[OsString]) -> Result<TargetRuntime, String> {
    let Some(program) = command.first() else {
        return Err("launch requires a command after --".to_string());
    };
    let path = std::env::var_os("PATH");
    resolve_target_runtime(program, path.as_ref().map(|value| value.as_os_str()))
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

#[derive(Default)]
pub(super) struct TlsSyncInjection {
    launch: Option<SyncLaunch>,
}

impl super::LaunchInjection for TlsSyncInjection {
    fn prepare(
        &mut self,
        command: &mut Vec<OsString>,
        context: super::PreparationContext<'_>,
    ) -> Result<super::InjectionResources, String> {
        let mut launch = sync_launch(
            context.client,
            context.request_id,
            command.clone(),
            context.config,
            context.agent_commands,
            context.timing,
        )?;
        let event_fd = if launch.sync_runtime_required {
            Some(InheritableSuppressedFd::connect_unix_socket(
                &context.config.sync_event_socket_path,
                SuppressedFdPurpose::TlsSyncEvent,
            )?)
        } else {
            None
        };
        *command = std::mem::take(&mut launch.command);
        let direct_probe_plans = std::mem::take(&mut launch.direct_probe_plans)
            .into_iter()
            .map(|plan| LaunchTlsProbePlan {
                target: plan.target,
                target_identity: plan.target_identity,
                binary: plan.binary,
                binary_identity: plan.binary_identity,
                provider: plan.provider,
                points: plan.points,
            })
            .collect();
        self.launch = Some(launch);
        Ok(super::InjectionResources {
            event_fd,
            direct_probe_plans,
        })
    }

    fn envs(
        &self,
        context: super::EnvironmentContext<'_>,
    ) -> Result<Vec<(OsString, OsString)>, String> {
        let launch = self
            .launch
            .as_ref()
            .ok_or_else(|| "TLS sync injection has not been prepared".to_string())?;
        env::sync_launch_envs(
            context.trace_id,
            context.config,
            context.socket_max_segment_bytes,
            launch,
            context.event_fd,
            context.timing,
        )
    }
}

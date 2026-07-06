//! Top-level command execution for the daemon operator binary.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use config_core::capture_profile::DeploymentPermissions;
use config_core::daemon::{
    EnforcementBuiltinRuleConfig, EnforcementConfig, OperatorConfig, OperatorConfigInitStatus,
    StartupPluginFailurePolicy, StartupPluginLoadConfig,
};
use control_contract::command::{
    ControlCommand, PluginCommandCommand, PluginListCommand, PluginLoadCommand,
    PluginStatusCommand, PluginUnloadCommand,
};
use control_contract::reply::ControlReply;
use daemon::{DaemonProfileRegistry, LocalDaemonServer, resolve_ebpf_collector_config};
use model_core::ids::RequestId;
use plugin_system::PluginInstanceStatus;
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use crate::args::{AcTraildCommand, PluginCommand, parse_args};
use crate::plugin_registry;
use crate::process::{
    DaemonProcessState, remove_runtime_file, start_daemon, status_daemon, stop_daemon,
    write_pid_file,
};
use crate::signals;

pub fn run_from_env() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        AcTraildCommand::Init {
            config_path,
            force,
            patch_path,
        } => initialize_operator_config(&config_path, force, patch_path.as_deref()),
        AcTraildCommand::Run { config_path } => {
            let config = OperatorConfig::load(&config_path)?;
            run_foreground(&config_path, &config)
        }
        AcTraildCommand::Start { config_path } => {
            let config = OperatorConfig::load(&config_path)?;
            start_daemon(&config_path, &config)
        }
        AcTraildCommand::Stop { config_path } => {
            let config = OperatorConfig::load(&config_path)?;
            stop_daemon(&config)
        }
        AcTraildCommand::Restart { config_path } => {
            let config = OperatorConfig::load(&config_path)?;
            stop_daemon(&config)?;
            start_daemon(&config_path, &config)
        }
        AcTraildCommand::Status { config_path } => {
            let config = OperatorConfig::load(&config_path)?;
            match status_daemon(&config)? {
                DaemonProcessState::Running { pid } => {
                    println!(
                        "actraild running pid={} socket={}",
                        pid,
                        config.socket_path.display()
                    );
                }
                DaemonProcessState::Stopped => {
                    println!("actraild stopped");
                }
                DaemonProcessState::StalePid { pid } => {
                    println!(
                        "actraild stale pid_file={} pid={}",
                        config.pid_file.display(),
                        pid
                    );
                }
                DaemonProcessState::StaleSocket => {
                    println!("actraild stale socket={}", config.socket_path.display());
                }
            }
            Ok(())
        }
        AcTraildCommand::Plugin {
            config_path,
            command,
        } => {
            let config = OperatorConfig::load(&config_path)?;
            run_plugin_command(&config_path, &config, command)
        }
    }
}

fn initialize_operator_config(
    path: &Path,
    force: bool,
    patch_path: Option<&Path>,
) -> Result<(), String> {
    match initialize_operator_config_file(path, force, patch_path)? {
        OperatorConfigInitStatus::Created => println!("initialized config {}", path.display()),
        OperatorConfigInitStatus::ExistingValid => {
            println!("config {} already exists and is valid", path.display());
        }
        OperatorConfigInitStatus::Overwritten => {
            println!("overwrote config {}", path.display());
        }
    }
    Ok(())
}

fn initialize_operator_config_file(
    path: &Path,
    force: bool,
    patch_path: Option<&Path>,
) -> Result<OperatorConfigInitStatus, String> {
    let existed = path.exists();
    if existed && !force {
        if let Some(patch_path) = patch_path {
            return Err(format!(
                "config {} already exists; pass --force to rewrite it with patch {}",
                path.display(),
                patch_path.display()
            ));
        }
        OperatorConfig::load(path)
            .map_err(|error| format!("validate config {}: {error}", path.display()))?;
        return Ok(OperatorConfigInitStatus::ExistingValid);
    }
    let mut config = OperatorConfig::init()?;
    if let Some(patch_path) = patch_path {
        config = config.patch_file(patch_path)?;
    }
    config.dump_to_path(path, force)?;
    Ok(if existed {
        OperatorConfigInitStatus::Overwritten
    } else {
        OperatorConfigInitStatus::Created
    })
}

fn run_foreground(config_path: &Path, config: &OperatorConfig) -> Result<(), String> {
    signals::install_shutdown_handlers()?;
    write_pid_file(&config.pid_file, std::process::id())?;
    let pid_written = true;
    let enforcement = enforcement_with_builtin_rules(config_path, config)?;
    let ebpf_resolution = resolve_ebpf_collector_config(config.ebpf_config.clone());
    if let Some(detail) = &ebpf_resolution.degrade_detail {
        tracing::warn!(detail = %detail, "actraild ebpf auto-degraded");
    }
    let mut profiles = DaemonProfileRegistry::new();
    profiles.insert_capture_profile(config.capture_profile.clone());
    for permissions in DeploymentPermissions::ALL {
        profiles.insert_launch_profile(config.capture_profile.for_permissions(permissions));
    }
    let mut server = match &config.provider_rule_set {
        Some(provider_rule_set) => LocalDaemonServer::build_with_provider_rule_set(
            &config.storage,
            profiles,
            ebpf_resolution.config.clone(),
            config.payload_config.clone(),
            config.active_trace_max,
            config.diagnostic_log_level,
            config.seccomp_notify.clone(),
            config.process_seccomp.clone(),
            config.agent_invocation.clone(),
            config.semantic_retention.clone(),
            config.file_observation.clone(),
            config.application_protocol.clone(),
            config.resource_metrics.clone(),
            config.trace_finalization,
            config.workload_diagnostics.clone(),
            config.export_runtime.clone(),
            enforcement.clone(),
            config.command_control.clone(),
            config.network_control.clone(),
            provider_rule_set,
        ),
        None => LocalDaemonServer::build(
            &config.storage,
            profiles,
            ebpf_resolution.config.clone(),
            config.payload_config.clone(),
            config.active_trace_max,
            config.diagnostic_log_level,
            config.seccomp_notify.clone(),
            config.process_seccomp.clone(),
            config.agent_invocation.clone(),
            config.semantic_retention.clone(),
            config.file_observation.clone(),
            config.application_protocol.clone(),
            config.resource_metrics.clone(),
            config.trace_finalization,
            config.workload_diagnostics.clone(),
            config.export_runtime.clone(),
            enforcement.clone(),
            config.command_control.clone(),
            config.network_control.clone(),
        ),
    }
    .map_err(|error| {
        cleanup_pid_file(config, pid_written).unwrap_or_else(|cleanup_error| {
            tracing::warn!(
                error = %cleanup_error,
                "daemon runtime cleanup failed after build error"
            );
        });
        format!("daemon build failed: {}: {}", error.code, error.message)
    })?;
    if let Err(error) = load_configured_startup_plugins(&mut server, config) {
        cleanup_pid_file(config, pid_written).unwrap_or_else(|cleanup_error| {
            tracing::warn!(
                error = %cleanup_error,
                "daemon runtime cleanup failed after startup plugin error"
            );
        });
        return Err(error);
    }
    if let Err(error) = load_persistent_plugins(&mut server, config_path) {
        cleanup_pid_file(config, pid_written).unwrap_or_else(|cleanup_error| {
            tracing::warn!(
                error = %cleanup_error,
                "daemon runtime cleanup failed after persistent plugin error"
            );
        });
        return Err(error);
    }

    let mut socket_bound = false;
    let result = server.serve_forever_until(
        &config.socket_path,
        config.socket_permissions,
        config.control_pending_connection_max,
        signals::shutdown_requested,
        || {
            socket_bound = true;
            println!(
                "daemon listening socket={} storage={}",
                config.socket_path.display(),
                config.storage.path().display()
            );
            Ok(())
        },
    );
    let cleanup = if socket_bound {
        cleanup_runtime_files(config, pid_written)
    } else if pid_written {
        cleanup_pid_file(config, pid_written)
    } else {
        Ok(())
    };
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(format!(
            "daemon run failed: {}: {}",
            error.stage, error.message
        )),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "daemon run failed: {}: {}; cleanup failed: {}",
            error.stage, error.message, cleanup_error
        )),
    }
}

fn run_plugin_command(
    config_path: &Path,
    config: &OperatorConfig,
    command: PluginCommand,
) -> Result<(), String> {
    match command {
        PluginCommand::Load {
            manifest_path,
            plugin_config_path,
            instance_id,
            host_grants,
            persist,
        } => {
            validate_plugin_instance_id(&instance_id)?;
            let reply = send_control_command(
                config,
                ControlCommand::PluginLoad(PluginLoadCommand {
                    request_id: RequestId::new(1),
                    manifest_path: manifest_path.display().to_string(),
                    plugin_config_path: plugin_config_path
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    instance_id: instance_id.clone(),
                    host_grants: host_grants.clone(),
                }),
            )?;
            let ControlReply::PluginStatus(status) = reply else {
                return Err("daemon returned unexpected plugin load reply".to_string());
            };
            if persist {
                if let Err(error) = plugin_registry::persist_instance(
                    config_path,
                    &manifest_path,
                    plugin_config_path.as_deref(),
                    &status.instance_id,
                    &host_grants,
                ) {
                    return Err(rollback_loaded_plugin(config, &status.instance_id, error));
                }
            }
            println!("loaded instance={}", status.instance_id);
            println!("warnings={}", printable_warnings(&status.warnings));
            Ok(())
        }
        PluginCommand::Unload {
            instance_id,
            persist,
        } => {
            validate_plugin_instance_id(&instance_id)?;
            let reply = send_control_command(
                config,
                ControlCommand::PluginUnload(PluginUnloadCommand {
                    request_id: RequestId::new(1),
                    instance_id,
                }),
            )?;
            let ControlReply::PluginStatus(status) = reply else {
                return Err("daemon returned unexpected plugin unload reply".to_string());
            };
            if persist {
                plugin_registry::remove_instance(config_path, &status.instance_id)?;
            }
            println!("unloaded instance={}", status.instance_id);
            Ok(())
        }
        PluginCommand::List => {
            let reply = send_control_command(
                config,
                ControlCommand::PluginList(PluginListCommand {
                    request_id: RequestId::new(1),
                }),
            )?;
            let ControlReply::PluginList(items) = reply else {
                return Err("daemon returned unexpected plugin list reply".to_string());
            };
            print_plugin_list(&items);
            Ok(())
        }
        PluginCommand::Status { instance_id } => {
            validate_plugin_instance_id(&instance_id)?;
            let reply = send_control_command(
                config,
                ControlCommand::PluginStatus(PluginStatusCommand {
                    request_id: RequestId::new(1),
                    instance_id,
                }),
            )?;
            let ControlReply::PluginStatus(status) = reply else {
                return Err("daemon returned unexpected plugin status reply".to_string());
            };
            print_plugin_status(&status);
            Ok(())
        }
        PluginCommand::Cmd { instance_id, argv } => {
            validate_plugin_instance_id(&instance_id)?;
            if argv.is_empty() {
                return Err("plugin_command: plugin argv must not be empty".to_string());
            }
            let reply = send_control_command(
                config,
                ControlCommand::PluginCommand(PluginCommandCommand {
                    request_id: RequestId::new(1),
                    instance_id,
                    argv,
                }),
            )?;
            let ControlReply::PluginCommand(reply) = reply else {
                return Err("daemon returned unexpected plugin command reply".to_string());
            };
            if !reply.stdout.is_empty() {
                print!("{}", reply.stdout);
            }
            if !reply.stderr.is_empty() {
                eprint!("{}", reply.stderr);
            }
            if reply.exit_code == 0 {
                Ok(())
            } else {
                Err(format!(
                    "plugin command for {} exited with {}",
                    reply.instance_id, reply.exit_code
                ))
            }
        }
    }
}

fn enforcement_with_builtin_rules(
    config_path: &Path,
    config: &OperatorConfig,
) -> Result<EnforcementConfig, String> {
    let mut enforcement = config.enforcement.clone();
    append_builtin_rule(&mut enforcement, "actrail.self.config", config_path)?;
    append_builtin_rule(&mut enforcement, "actrail.self.pid", &config.pid_file)?;
    append_builtin_rule(&mut enforcement, "actrail.self.socket", &config.socket_path)?;
    append_builtin_rule(&mut enforcement, "actrail.self.log", &config.log_path)?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.storage",
        config.storage.path(),
    )?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.storage-wal",
        &path_with_suffix(config.storage.path(), "-wal"),
    )?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.storage-shm",
        &path_with_suffix(config.storage.path(), "-shm"),
    )?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.enforcement-rules",
        &config.enforcement.rules_path,
    )?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.command-control-rules",
        &config.command_control.rules_path,
    )?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.network-control-rules",
        &config.network_control.rules_path,
    )?;
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.payload-tls-sync-socket",
        &config.payload_config.tls.sync_event_socket_path,
    )?;
    if let Some(provider_rule_set) = &config.provider_rule_set {
        append_builtin_rule(
            &mut enforcement,
            "actrail.self.provider-rules",
            &provider_rule_set.rules_path,
        )?;
    }
    append_builtin_rule(
        &mut enforcement,
        "actrail.self.plugin-registry",
        &plugin_registry::registry_path(config_path)?,
    )?;
    Ok(enforcement)
}

fn append_builtin_rule(
    enforcement: &mut EnforcementConfig,
    rule_id: &str,
    path: &Path,
) -> Result<(), String> {
    enforcement
        .builtin_rules
        .push(EnforcementBuiltinRuleConfig {
            rule_id: rule_id.to_string(),
            path: absolute_runtime_path(path)?.display().to_string(),
        });
    Ok(())
}

fn absolute_runtime_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|current_dir| current_dir.join(path))
        .map_err(|error| format!("resolve current directory for {}: {error}", path.display()))
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(path.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

fn validate_plugin_instance_id(instance_id: &str) -> Result<(), String> {
    if instance_id.trim().is_empty() {
        return Err("plugin_command: plugin instance id must not be empty".to_string());
    }
    Ok(())
}

fn rollback_loaded_plugin(
    config: &OperatorConfig,
    instance_id: &str,
    persist_error: String,
) -> String {
    let rollback = send_control_command(
        config,
        ControlCommand::PluginUnload(PluginUnloadCommand {
            request_id: RequestId::new(1),
            instance_id: instance_id.to_string(),
        }),
    );
    match rollback {
        Ok(_) => format!(
            "persistent plugin registry update failed after loading {instance_id}; rolled back runtime load: {persist_error}"
        ),
        Err(rollback_error) => format!(
            "persistent plugin registry update failed after loading {instance_id}: {persist_error}; runtime rollback also failed: {rollback_error}"
        ),
    }
}

fn load_configured_startup_plugins(
    server: &mut LocalDaemonServer,
    config: &OperatorConfig,
) -> Result<(), String> {
    if !config.startup_plugins.enabled {
        return Ok(());
    }
    let mut request_id = 1_u64;
    for plugin in config
        .startup_plugins
        .load
        .iter()
        .filter(|plugin| plugin.enabled)
    {
        let policy = plugin
            .failure_policy
            .unwrap_or(config.startup_plugins.failure_policy);
        let instance_id = plugin.instance_id.clone();
        let command = startup_plugin_load_command(plugin, request_id);
        request_id = request_id
            .checked_add(1)
            .ok_or_else(|| "startup plugin request id overflow".to_string())?;
        match server.load_plugin(command) {
            Ok(status) => {
                tracing::info!(
                    instance = %status.instance_id,
                    "startup plugin loaded"
                );
                if !status.warnings.is_empty() {
                    tracing::warn!(
                        instance = %status.instance_id,
                        warnings = %printable_warnings(&status.warnings),
                        "startup plugin loaded with warnings"
                    );
                }
            }
            Err(error) => {
                let message = format!(
                    "startup plugin {instance_id} load failed: {}: {}",
                    error.code, error.message
                );
                match policy {
                    StartupPluginFailurePolicy::FailFast => {
                        tracing::error!(
                            instance = %instance_id,
                            code = %error.code,
                            error = %error.message,
                            "startup plugin load failed"
                        );
                        return Err(message);
                    }
                    StartupPluginFailurePolicy::Continue => {
                        tracing::warn!(
                            instance = %instance_id,
                            code = %error.code,
                            error = %error.message,
                            "startup plugin load failed; continuing"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn startup_plugin_load_command(
    plugin: &StartupPluginLoadConfig,
    request_id: u64,
) -> PluginLoadCommand {
    PluginLoadCommand {
        request_id: RequestId::new(request_id),
        manifest_path: plugin.manifest_path.display().to_string(),
        plugin_config_path: plugin
            .plugin_config_path
            .as_ref()
            .map(|path| path.display().to_string()),
        instance_id: plugin.instance_id.clone(),
        host_grants: plugin.host_grants.clone(),
    }
}

fn load_persistent_plugins(
    server: &mut LocalDaemonServer,
    config_path: &Path,
) -> Result<(), String> {
    for command in plugin_registry::startup_load_commands(config_path)? {
        let instance_id = command.instance_id.clone();
        server.load_plugin(command).map_err(|error| {
            format!(
                "persistent plugin {instance_id} load failed: {}: {}",
                error.code, error.message
            )
        })?;
    }
    Ok(())
}

fn send_control_command(
    config: &OperatorConfig,
    command: ControlCommand,
) -> Result<ControlReply, String> {
    let transport = UdsSocketTransport::new(config.socket_path.clone());
    let mut client = UdsControlClient::new(transport);
    client
        .send(command)
        .map_err(|error| format!("control {}: {}", error.code, error.message))
}

fn print_plugin_list(items: &[PluginInstanceStatus]) {
    println!("INSTANCE\tPLUGIN\tPURPOSE\tRUNTIME\tSTATE\tHOST_GRANTS\tQUEUE\tWARNINGS");
    for item in items {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            item.instance_id,
            item.plugin_id,
            item.purpose.as_str(),
            item.runtime.as_str(),
            item.state.as_str(),
            printable_host_grants(&item.host_grants),
            printable_queue(item.queue_depth, item.queue_capacity),
            printable_warnings(&item.warnings)
        );
    }
}

fn print_plugin_status(status: &PluginInstanceStatus) {
    println!("instance={}", status.instance_id);
    println!("plugin_id={}", status.plugin_id);
    println!("purpose={}", status.purpose.as_str());
    println!("runtime={}", status.runtime.as_str());
    println!("state={}", status.state.as_str());
    println!("host_grants={}", printable_host_grants(&status.host_grants));
    println!("queue_depth={}", printable_optional_u64(status.queue_depth));
    println!(
        "queue_capacity={}",
        printable_optional_u32(status.queue_capacity)
    );
    println!("observed_records={}", status.observed_records);
    println!("dropped_records={}", status.dropped_records);
    let payload_read = status.hostcall_metrics.payload_read;
    println!("payload_read_calls={}", payload_read.calls);
    println!("payload_read_bytes={}", payload_read.bytes);
    println!("payload_read_denied={}", payload_read.denied);
    println!("payload_read_not_found={}", payload_read.not_found);
    println!("payload_read_invalid={}", payload_read.invalid);
    println!("payload_read_too_large={}", payload_read.too_large);
    println!("payload_read_truncated={}", payload_read.truncated);
    println!(
        "payload_read_latency_total_ns={}",
        payload_read.latency_total_ns
    );
    println!(
        "payload_read_latency_max_ns={}",
        payload_read.latency_max_ns
    );
    println!(
        "last_error={}",
        status.last_error.as_deref().unwrap_or("none")
    );
    println!("warnings={}", printable_warnings(&status.warnings));
}

fn printable_queue(queue_depth: Option<u64>, queue_capacity: Option<u32>) -> String {
    match (queue_depth, queue_capacity) {
        (Some(depth), Some(capacity)) => format!("{depth}/{capacity}"),
        _ => "none".to_string(),
    }
}

fn printable_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn printable_optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn printable_host_grants(host_grants: &[String]) -> String {
    if host_grants.is_empty() {
        "none".to_string()
    } else {
        host_grants.join(",")
    }
}

fn printable_warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        "none".to_string()
    } else {
        warnings.join(";")
    }
}

fn cleanup_runtime_files(config: &OperatorConfig, pid_written: bool) -> Result<(), String> {
    cleanup_pid_file(config, pid_written)?;
    remove_runtime_file(&config.socket_path)?;
    if config.payload_config.tls.capture_backend.is_sync() {
        remove_runtime_file(&config.payload_config.tls.sync_event_socket_path)?;
    }
    Ok(())
}

fn cleanup_pid_file(config: &OperatorConfig, pid_written: bool) -> Result<(), String> {
    if pid_written {
        remove_runtime_file(&config.pid_file)?;
    }
    Ok(())
}

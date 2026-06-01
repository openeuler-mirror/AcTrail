//! Top-level command execution for the daemon operator binary.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use config_core::daemon::{OPERATOR_CONFIG_TEMPLATE, OperatorConfig};
use daemon::{DaemonProfileRegistry, DaemonRunError, LocalDaemonServer};

use crate::args::{AcTraildCommand, parse_args};
use crate::process::{
    DaemonProcessState, remove_runtime_file, start_daemon, status_daemon, stop_daemon,
    write_pid_file,
};
use crate::signals;

pub fn run_from_env() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        AcTraildCommand::InitConfig { output_path } => write_operator_config(&output_path),
        AcTraildCommand::Run { config_path } => {
            let config = OperatorConfig::load(&config_path)?;
            run_foreground(&config)
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
    }
}

fn write_operator_config(path: &Path) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create config directory {}: {error}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create config {}: {error}", path.display()))?;
    file.write_all(OPERATOR_CONFIG_TEMPLATE.as_bytes())
        .map_err(|error| format!("write config {}: {error}", path.display()))?;
    println!("wrote config {}", path.display());
    Ok(())
}

fn run_foreground(config: &OperatorConfig) -> Result<(), String> {
    signals::install_shutdown_handlers()?;
    let mut profiles = DaemonProfileRegistry::new();
    profiles.insert_capture_profile(config.capture_profile.clone());
    let mut server = match &config.provider_rule_set {
        Some(provider_rule_set) => LocalDaemonServer::build_with_provider_rule_set(
            &config.storage_path,
            profiles,
            config.ebpf_config.clone(),
            config.diagnostic_log_level,
            config.seccomp_notify.clone(),
            config.process_seccomp.clone(),
            config.agent_invocation.clone(),
            config.application_protocol.clone(),
            config.resource_metrics.clone(),
            config.live_otel_export.clone(),
            config.enforcement.clone(),
            provider_rule_set,
        ),
        None => LocalDaemonServer::build(
            &config.storage_path,
            profiles,
            config.ebpf_config.clone(),
            config.diagnostic_log_level,
            config.seccomp_notify.clone(),
            config.process_seccomp.clone(),
            config.agent_invocation.clone(),
            config.application_protocol.clone(),
            config.resource_metrics.clone(),
            config.live_otel_export.clone(),
            config.enforcement.clone(),
        ),
    }
    .map_err(|error| format!("daemon build failed: {}: {}", error.code, error.message))?;

    let mut socket_bound = false;
    let mut pid_written = false;
    let result = server.serve_forever_until(
        &config.socket_path,
        config.socket_permissions,
        signals::shutdown_requested,
        || {
            socket_bound = true;
            write_pid_file(&config.pid_file, std::process::id()).map_err(run_error)?;
            pid_written = true;
            println!(
                "daemon listening socket={} storage={}",
                config.socket_path.display(),
                config.storage_path.display()
            );
            Ok(())
        },
    );
    let cleanup = if socket_bound {
        cleanup_runtime_files(config, pid_written)
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

fn cleanup_runtime_files(config: &OperatorConfig, pid_written: bool) -> Result<(), String> {
    if pid_written {
        remove_runtime_file(&config.pid_file)?;
    }
    remove_runtime_file(&config.socket_path)
}

fn run_error(error: String) -> DaemonRunError {
    DaemonRunError {
        stage: "ready".to_string(),
        message: error,
    }
}

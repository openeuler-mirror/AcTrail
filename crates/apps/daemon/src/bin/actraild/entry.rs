//! Top-level command execution for the daemon operator binary.

use std::path::Path;

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};
use daemon::{DaemonProfileRegistry, DaemonRunError, LocalDaemonServer};

use crate::args::{AcTraildCommand, parse_args};
use crate::process::{
    DaemonProcessState, remove_runtime_file, start_daemon, status_daemon, stop_daemon,
    write_pid_file,
};
use crate::signals;

pub fn run_from_env() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        AcTraildCommand::Init { config_path, force } => {
            initialize_operator_config(&config_path, force)
        }
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

fn initialize_operator_config(path: &Path, force: bool) -> Result<(), String> {
    match OperatorConfig::initialize(path, force)? {
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

fn run_foreground(config: &OperatorConfig) -> Result<(), String> {
    signals::install_shutdown_handlers()?;
    let mut profiles = DaemonProfileRegistry::new();
    profiles.insert_capture_profile(config.capture_profile.clone());
    let mut server = match &config.provider_rule_set {
        Some(provider_rule_set) => LocalDaemonServer::build_with_provider_rule_set(
            &config.storage,
            profiles,
            config.ebpf_config.clone(),
            config.payload_config.clone(),
            config.diagnostic_log_level,
            config.seccomp_notify.clone(),
            config.process_seccomp.clone(),
            config.agent_invocation.clone(),
            config.semantic_retention.clone(),
            config.file_observation.clone(),
            config.application_protocol.clone(),
            config.resource_metrics.clone(),
            config.export_runtime.clone(),
            config.enforcement.clone(),
            provider_rule_set,
        ),
        None => LocalDaemonServer::build(
            &config.storage,
            profiles,
            config.ebpf_config.clone(),
            config.payload_config.clone(),
            config.diagnostic_log_level,
            config.seccomp_notify.clone(),
            config.process_seccomp.clone(),
            config.agent_invocation.clone(),
            config.semantic_retention.clone(),
            config.file_observation.clone(),
            config.application_protocol.clone(),
            config.resource_metrics.clone(),
            config.export_runtime.clone(),
            config.enforcement.clone(),
        ),
    }
    .map_err(|error| format!("daemon build failed: {}: {}", error.code, error.message))?;

    let mut socket_bound = false;
    let mut pid_written = false;
    let result = server.serve_forever_until(
        &config.socket_path,
        config.socket_permissions,
        config.control_pending_connection_max,
        signals::shutdown_requested,
        || {
            socket_bound = true;
            write_pid_file(&config.pid_file, std::process::id()).map_err(run_error)?;
            pid_written = true;
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
    remove_runtime_file(&config.socket_path)?;
    if config.payload_config.tls.capture_backend.is_sync() {
        remove_runtime_file(&config.payload_config.tls.sync_event_socket_path)?;
    }
    Ok(())
}

fn run_error(error: String) -> DaemonRunError {
    DaemonRunError {
        stage: "ready".to_string(),
        message: error,
    }
}

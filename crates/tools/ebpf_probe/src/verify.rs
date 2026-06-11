//! Live verification against the real daemon, eBPF collector, and SQLite store.

#[path = "verify/database.rs"]
mod database;

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use config_core::capture_profile::CaptureProfile;
use config_core::daemon::{
    EbpfCollectorConfig, OPERATOR_CONFIG_TEMPLATE, OperatorConfig, PayloadConfig,
};
use config_core::provider_rules::ProviderRuleSetConfig;
use control_contract::command::{ControlCommand, TrackAddCommand};
use control_contract::reply::ControlReply;
use daemon::{DaemonProfileRegistry, LocalDaemonServer};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{ProfileName, RequestId, TraceId, TraceName};

use crate::args::{LiveVerificationConfig, workload_from_live_config};
use crate::report::LiveVerificationReport;

pub fn run_live_verification(
    config: LiveVerificationConfig,
) -> Result<LiveVerificationReport, String> {
    prepare_paths(&config)?;

    let provider_rule_set = ProviderRuleSetConfig {
        rules_path: config.provider_rules_path.clone(),
        unknown_provider_label: config.provider_unknown_provider_label.clone(),
    };
    let seccomp_defaults = OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE)
        .map_err(|error| format!("parse built-in seccomp defaults: {error}"))?;
    let mut server = LocalDaemonServer::build_with_provider_rule_set(
        &config.storage_path,
        seccomp_defaults.storage_busy_timeout_ms,
        verification_profiles(&config),
        EbpfCollectorConfig {
            enabled: true,
            memlock_rlimit: config.memlock_rlimit,
            tracked_process_max_entries: config.tracked_process_max_entries,
            pending_operation_max_entries: config.pending_operation_max_entries,
            event_ring_buffer_max_bytes: config.event_ring_buffer_max_bytes,
            file_path_capture_enabled: config.file_path_capture_enabled,
            file_path_max_bytes: config.file_path_max_bytes,
        },
        PayloadConfig {
            tls: config.payload_tls.clone(),
            stdio: config.payload_stdio.clone(),
            socket: config.payload_socket.clone(),
        },
        seccomp_defaults.diagnostic_log_level,
        seccomp_defaults.seccomp_notify,
        seccomp_defaults.process_seccomp,
        seccomp_defaults.agent_invocation,
        config.application_protocol.clone(),
        config.resource_metrics.clone(),
        seccomp_defaults.live_otel_export,
        config.enforcement.clone(),
        &provider_rule_set,
    )
    .map_err(|error| format!("daemon build failed: {}: {}", error.code, error.message))?;

    let mut child = spawn_workload(&config)?;
    let root_pid = child.id();
    let trace_id = match run_workload_under_trace(&mut server, &config, &mut child, root_pid) {
        Ok(trace_id) => trace_id,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };

    let final_snapshot = server.ebpf_debug_snapshot(root_pid).map_err(|error| {
        format!(
            "eBPF debug snapshot failed: {}: {}",
            error.code, error.message
        )
    })?;
    let debug_description = format!(
        "eBPF debug: active_binding_traces={}, last_raw_sample_count={}, tracked_trace_id={:?}, attached_programs={}",
        final_snapshot.active_binding_traces,
        final_snapshot.last_raw_sample_count,
        final_snapshot
            .tracked_trace_id
            .map(|trace_id| trace_id.get()),
        final_snapshot.attached_programs.join(",")
    );
    database::verify_database(
        &config.storage_path,
        trace_id,
        &config.provider_expected_provider,
        &config.stdio_continue_message,
        &config.stdio_stdout_message,
        &config.stdio_stderr_message,
        config.mmap.as_ref().map(|mmap| (mmap.length, mmap.offset)),
        config.resource_metrics.enabled,
        config.resource_metrics.include_system,
    )
    .map_err(|error| format!("{error}; {debug_description}"))
}

fn run_workload_under_trace(
    server: &mut LocalDaemonServer,
    config: &LiveVerificationConfig,
    child: &mut std::process::Child,
    root_pid: u32,
) -> Result<TraceId, String> {
    let trace_id = track_workload(server, config, root_pid)?;
    verify_runtime_binding(server, root_pid, trace_id)?;
    drain_resource_metrics_if_enabled(server, config)?;
    write_child_line(child, &config.stdio_stdin_message)?;
    read_child_until_events_ready(child, &config.stdio_stdout_message)?;
    server.drain_live_events().map_err(|error| {
        format!(
            "drain before close failed: {}: {}",
            error.code, error.message
        )
    })?;
    write_child_line(child, &config.stdio_continue_message)?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("workload exited with {status}"));
    }
    server
        .drain_live_events()
        .map_err(|error| format!("final drain failed: {}: {}", error.code, error.message))?;
    Ok(trace_id)
}

fn drain_resource_metrics_if_enabled(
    server: &mut LocalDaemonServer,
    config: &LiveVerificationConfig,
) -> Result<(), String> {
    if !config.resource_metrics.enabled {
        return Ok(());
    }
    std::thread::sleep(Duration::from_millis(config.resource_metrics.interval_ms));
    server.drain_live_events().map_err(|error| {
        format!(
            "resource metrics drain failed: {}: {}",
            error.code, error.message
        )
    })
}

fn prepare_paths(config: &LiveVerificationConfig) -> Result<(), String> {
    if config.storage_path.exists() {
        return Err(format!(
            "storage path already exists: {}",
            config.storage_path.display()
        ));
    }
    for path in [
        &config.fifo_path,
        &config.file_path,
        &config.mkdir_path,
        &config.rmdir_path,
        &config.rename_source_path,
        &config.rename_target_path,
        &config.unlink_path,
        &config.truncate_path,
    ] {
        if path.exists() {
            return Err(format!("workload path already exists: {}", path.display()));
        }
    }
    if let Some(mmap) = &config.mmap {
        if mmap.path.exists() {
            return Err(format!(
                "workload path already exists: {}",
                mmap.path.display()
            ));
        }
    }
    if let Some(parent) = config.storage_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    for path in [
        &config.fifo_path,
        &config.file_path,
        &config.mkdir_path,
        &config.rmdir_path,
        &config.rename_source_path,
        &config.rename_target_path,
        &config.unlink_path,
        &config.truncate_path,
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    if let Some(parent) = config.mmap.as_ref().and_then(|mmap| mmap.path.parent()) {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn verification_profiles(config: &LiveVerificationConfig) -> DaemonProfileRegistry {
    let mut profiles = DaemonProfileRegistry::new();
    let mut capabilities = vec![
        CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
        CapabilityRequest::new(Capability::NetTransport, RequestMode::Required),
        CapabilityRequest::new(Capability::FsAccessBasic, RequestMode::Required),
        CapabilityRequest::new(Capability::IpcUnixSocket, RequestMode::Required),
        CapabilityRequest::new(Capability::IpcPipeFifo, RequestMode::Required),
        CapabilityRequest::new(Capability::StdioChunk, RequestMode::Required),
    ];
    if config.mmap.is_some() {
        capabilities.push(CapabilityRequest::new(
            Capability::FsMmap,
            RequestMode::Required,
        ));
    }
    if config.application_protocol.enabled && config.payload_tls.enabled {
        capabilities.push(CapabilityRequest::new(
            Capability::TlsPlaintextPayload,
            RequestMode::Required,
        ));
    }
    if config.application_protocol.enabled && config.payload_socket.enabled {
        capabilities.push(CapabilityRequest::new(
            Capability::SocketPlaintextPayload,
            RequestMode::Required,
        ));
    }
    if config.application_protocol.http1_enabled {
        capabilities.push(CapabilityRequest::new(
            Capability::NetApplicationPlaintextHttp,
            RequestMode::Required,
        ));
    }
    if config.application_protocol.http2_enabled {
        capabilities.push(CapabilityRequest::new(
            Capability::NetApplicationHttp2Frames,
            RequestMode::Required,
        ));
    }
    if config.resource_metrics.enabled {
        capabilities.push(CapabilityRequest::new(
            Capability::ResourceMetrics,
            RequestMode::Required,
        ));
    }
    profiles.insert_capture_profile(CaptureProfile::new(
        ProfileName::new(config.profile_name.clone()),
        capabilities,
    ));
    profiles
}

fn spawn_workload(config: &LiveVerificationConfig) -> Result<std::process::Child, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(executable);
    command.arg("workload");

    let workload = workload_from_live_config(config);
    command
        .arg("--exec-path")
        .arg(&workload.exec_path)
        .arg("--listen-addr")
        .arg(&workload.listen_addr)
        .arg("--client-message")
        .arg(&workload.client_message)
        .arg("--server-message")
        .arg(&workload.server_message)
        .arg("--stdio-stdin-message")
        .arg(&workload.stdio_stdin_message)
        .arg("--stdio-continue-message")
        .arg(&workload.stdio_continue_message)
        .arg("--stdio-stdout-message")
        .arg(&workload.stdio_stdout_message)
        .arg("--stdio-stderr-message")
        .arg(&workload.stdio_stderr_message)
        .arg("--process-signal-number")
        .arg(workload.process_signal_number.to_string())
        .arg("--pipe-message")
        .arg(&workload.pipe_message)
        .arg("--fifo-path")
        .arg(&workload.fifo_path)
        .arg("--fifo-mode")
        .arg(format!("{:o}", workload.fifo_mode))
        .arg("--file-path")
        .arg(&workload.file_path)
        .arg("--file-message")
        .arg(&workload.file_message);
    if let Some(mmap) = &workload.mmap {
        command
            .arg("--mmap-path")
            .arg(&mmap.path)
            .arg("--mmap-message")
            .arg(&mmap.message)
            .arg("--mmap-length")
            .arg(mmap.length.to_string())
            .arg("--mmap-offset")
            .arg(mmap.offset.to_string());
    }
    command
        .arg("--mkdir-path")
        .arg(&workload.mkdir_path)
        .arg("--rmdir-path")
        .arg(&workload.rmdir_path)
        .arg("--rename-source-path")
        .arg(&workload.rename_source_path)
        .arg("--rename-target-path")
        .arg(&workload.rename_target_path)
        .arg("--unlink-path")
        .arg(&workload.unlink_path)
        .arg("--truncate-path")
        .arg(&workload.truncate_path)
        .arg("--unix-message")
        .arg(&workload.unix_message)
        .arg("--directory-mode")
        .arg(format!("{:o}", workload.directory_mode))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())
}

fn track_workload(
    server: &mut LocalDaemonServer,
    config: &LiveVerificationConfig,
    root_pid: u32,
) -> Result<TraceId, String> {
    let reply = send_control(
        server,
        ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(config.request_id_start),
            root_pid,
            display_name: TraceName::new(config.trace_name.clone()),
            profile_name: ProfileName::new(config.profile_name.clone()),
            tags: BTreeSet::new(),
            launch_mode: false,
        }),
    )?;
    let ControlReply::TrackAdded(reply) = reply else {
        return Err("track add returned unexpected reply".to_string());
    };
    Ok(reply.trace_id)
}

fn verify_runtime_binding(
    server: &mut LocalDaemonServer,
    root_pid: u32,
    trace_id: TraceId,
) -> Result<(), String> {
    let snapshot = server.ebpf_debug_snapshot(root_pid).map_err(|error| {
        format!(
            "eBPF debug snapshot failed: {}: {}",
            error.code, error.message
        )
    })?;
    if snapshot.attached_programs.is_empty() {
        return Err("eBPF runtime did not attach any programs".to_string());
    }
    if snapshot.active_binding_traces == 0 {
        return Err("eBPF collector has no active binding traces after track-add".to_string());
    }
    if snapshot.tracked_trace_id != Some(trace_id) {
        return Err(format!(
            "tracked_traces missing root pid {root_pid}; expected trace {}, observed {:?}; attached programs: {}",
            trace_id.get(),
            snapshot.tracked_trace_id.map(|trace_id| trace_id.get()),
            snapshot.attached_programs.join(",")
        ));
    }
    Ok(())
}

fn send_control(
    server: &mut LocalDaemonServer,
    command: ControlCommand,
) -> Result<ControlReply, String> {
    let request = uds_control_transport::encode_command(&command);
    let response = server.handle_request(&request);
    uds_control_transport::decode_reply(&response)
        .map_err(|error| format!("decode reply failed: {}: {}", error.stage, error.message))?
        .map_err(|error| format!("control command failed: {}: {}", error.code, error.message))
}

fn write_child_line(child: &mut std::process::Child, line: &str) -> Result<(), String> {
    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| "workload stdin is unavailable".to_string())?;
    writeln!(stdin, "{line}").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())
}

fn read_child_until_events_ready(
    child: &mut std::process::Child,
    expected_stdout_payload: &str,
) -> Result<(), String> {
    let stdout = child
        .stdout
        .as_mut()
        .ok_or_else(|| "workload stdout is unavailable".to_string())?;
    let mut reader = BufReader::new(stdout);
    let mut observed_stdout_payload = false;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("workload stdout closed before events-ready".to_string());
        }
        let trimmed = line.trim_end();
        if trimmed == expected_stdout_payload {
            observed_stdout_payload = true;
        }
        if trimmed == "events-ready" {
            if observed_stdout_payload {
                return Ok(());
            }
            return Err(format!(
                "events-ready arrived before stdout payload {expected_stdout_payload}"
            ));
        }
    }
}

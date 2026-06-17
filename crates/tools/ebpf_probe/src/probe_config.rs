//! Config-file parsing for eBPF probe verification runs.

#[path = "probe_config/keys.rs"]
mod keys;
#[path = "probe_config/values.rs"]
mod values;

use std::fs;
use std::path::Path;

use config_core::daemon::{PayloadStdioStorageMode, SseDataPolicy};

use crate::args::{LiveVerificationConfig, WorkloadConfig};
use keys::{live_config_keys, workload_config_keys};
use values::ConfigValues;

pub fn load_live_verification_config(path: &Path) -> Result<LiveVerificationConfig, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read probe config {}: {error}", path.display()))?;
    let values = ConfigValues::parse(&raw, live_config_keys())?;
    let config = LiveVerificationConfig {
        source_config_path: Some(path.to_path_buf()),
        storage_path: values.required_path("storage_path")?,
        profile_name: values.required("profile_name")?,
        trace_name: values.required("trace_name")?,
        request_id_start: values.required_u64("request_id_start")?,
        memlock_rlimit: values.required_memlock_rlimit("memlock_rlimit")?,
        tracked_process_max_entries: values.required_positive_u32("tracked_process_max_entries")?,
        pending_operation_max_entries: values
            .required_positive_u32("pending_operation_max_entries")?,
        suppressed_fd_max_entries: values.required_positive_u32("suppressed_fd_max_entries")?,
        event_ring_buffer_max_bytes: values.required_positive_u32("event_ring_buffer_max_bytes")?,
        file_path_capture_enabled: values.required_bool("file_path_capture_enabled")?,
        file_path_max_bytes: values.required_positive_u32("file_path_max_bytes")?,
        payload_tls: values.payload_tls_config()?,
        payload_stdio: values.payload_stdio_config()?,
        payload_socket: values.payload_socket_config()?,
        application_protocol: values.application_protocol_config()?,
        resource_metrics: values.resource_metrics_config()?,
        enforcement: values.enforcement_config()?,
        process_signal_number: values.required_positive_u32("process_signal_number")?,
        exec_path: values.required_path("exec_path")?,
        listen_addr: values.required("listen_addr")?,
        client_message: values.required("client_message")?,
        server_message: values.required("server_message")?,
        stdio_stdin_message: values.required("stdio_stdin_message")?,
        stdio_continue_message: values.required("stdio_continue_message")?,
        stdio_stdout_message: values.required("stdio_stdout_message")?,
        stdio_stderr_message: values.required("stdio_stderr_message")?,
        pipe_message: values.required("pipe_message")?,
        fifo_path: values.required_path("fifo_path")?,
        fifo_mode: values.required_octal("fifo_mode")?,
        file_path: values.required_path("file_path")?,
        file_message: values.required("file_message")?,
        mmap: values.optional_mmap_config()?,
        mkdir_path: values.required_path("mkdir_path")?,
        rmdir_path: values.required_path("rmdir_path")?,
        rename_source_path: values.required_path("rename_source_path")?,
        rename_target_path: values.required_path("rename_target_path")?,
        unlink_path: values.required_path("unlink_path")?,
        truncate_path: values.required_path("truncate_path")?,
        unix_message: values.required("unix_message")?,
        directory_mode: values.required_octal("directory_mode")?,
        provider_rules_path: values.required_path("provider_rules_path")?,
        provider_unknown_provider_label: values.required("provider_unknown_provider_label")?,
        provider_expected_provider: values.required("provider_expected_provider")?,
    };
    validate_live_config(&config)?;
    Ok(config)
}

pub fn load_workload_config(path: &Path) -> Result<WorkloadConfig, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read workload config {}: {error}", path.display()))?;
    let values = ConfigValues::parse(&raw, workload_config_keys())?;
    Ok(WorkloadConfig {
        exec_path: values.required_path("exec_path")?,
        listen_addr: values.required("listen_addr")?,
        client_message: values.required("client_message")?,
        server_message: values.required("server_message")?,
        stdio_stdin_message: values.required("stdio_stdin_message")?,
        stdio_continue_message: values.required("stdio_continue_message")?,
        stdio_stdout_message: values.required("stdio_stdout_message")?,
        stdio_stderr_message: values.required("stdio_stderr_message")?,
        process_signal_number: values.required_positive_u32("process_signal_number")?,
        pipe_message: values.required("pipe_message")?,
        fifo_path: values.required_path("fifo_path")?,
        fifo_mode: values.required_octal("fifo_mode")?,
        file_path: values.required_path("file_path")?,
        file_message: values.required("file_message")?,
        mmap: values.optional_mmap_config()?,
        mkdir_path: values.required_path("mkdir_path")?,
        rmdir_path: values.required_path("rmdir_path")?,
        rename_source_path: values.required_path("rename_source_path")?,
        rename_target_path: values.required_path("rename_target_path")?,
        unlink_path: values.required_path("unlink_path")?,
        truncate_path: values.required_path("truncate_path")?,
        unix_message: values.required("unix_message")?,
        directory_mode: values.required_octal("directory_mode")?,
    })
}

fn validate_live_config(config: &LiveVerificationConfig) -> Result<(), String> {
    if !config.payload_stdio.enabled {
        return Err("verify-live requires payload_stdio_enabled = true".to_string());
    }
    if !(config.payload_stdio.capture_stdin
        && config.payload_stdio.capture_stdout
        && config.payload_stdio.capture_stderr)
    {
        return Err(
            "verify-live requires payload_stdio_capture_stdin/stdout/stderr = true".to_string(),
        );
    }
    if !(config.payload_stdio.stdin_storage_mode == PayloadStdioStorageMode::Full
        && config.payload_stdio.stdout_storage_mode == PayloadStdioStorageMode::Full
        && config.payload_stdio.stderr_storage_mode == PayloadStdioStorageMode::Full)
    {
        return Err(
            "verify-live requires payload_stdio_stdin/stdout/stderr_storage_mode = full"
                .to_string(),
        );
    }
    if !config.file_path_capture_enabled {
        return Err("verify-live requires file_path_capture_enabled = true".to_string());
    }
    if config.application_protocol.enabled
        && !(config.payload_tls.enabled || config.payload_socket.enabled)
    {
        return Err(
            "application_protocol_enabled requires payload_tls_enabled = true or payload_socket_enabled = true"
                .to_string(),
        );
    }
    if config.application_protocol.http1_enabled && !config.application_protocol.enabled {
        return Err(
            "application_protocol_http1_enabled requires application_protocol_enabled = true"
                .to_string(),
        );
    }
    if config.application_protocol.http2_enabled && !config.application_protocol.enabled {
        return Err(
            "application_protocol_http2_enabled requires application_protocol_enabled = true"
                .to_string(),
        );
    }
    if config.application_protocol.sse_enabled && !config.application_protocol.http1_enabled {
        return Err(
            "application_http_sse_enabled requires application_protocol_http1_enabled = true"
                .to_string(),
        );
    }
    if matches!(
        config.application_protocol.sse_data_policy,
        SseDataPolicy::Preview
    ) && !config.application_protocol.sse_enabled
    {
        return Err(
            "application_http_sse_data_policy = preview requires application_http_sse_enabled = true"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use config_core::daemon::PayloadStdioStorageMode;

    use super::{load_live_verification_config, load_workload_config};

    #[test]
    fn public_extended_observation_config_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("docs/examples/03.extended-observation-e2e/observation.conf");
        let config = load_live_verification_config(&path).expect("parse public example config");

        assert_eq!(config.profile_name, "actrail-extended-e2e");
        assert_eq!(config.trace_name, "actrail-extended-live");
        assert!(config.payload_stdio.enabled);
        assert!(config.payload_stdio.capture_stdin);
        assert!(config.payload_stdio.capture_stdout);
        assert!(config.payload_stdio.capture_stderr);
        assert_eq!(
            config.payload_stdio.stdin_storage_mode,
            PayloadStdioStorageMode::Full
        );
        assert_eq!(
            config.payload_stdio.stdout_storage_mode,
            PayloadStdioStorageMode::Full
        );
        assert_eq!(
            config.payload_stdio.stderr_storage_mode,
            PayloadStdioStorageMode::Full
        );
        assert!(config.mmap.is_some());
        assert_eq!(config.provider_expected_provider, "actrail-local-tcp");
    }

    #[test]
    fn public_extended_workload_config_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("docs/examples/03.extended-observation-e2e/workload.conf");
        let config = load_workload_config(&path).expect("parse public workload config");

        assert_eq!(config.stdio_stdin_message, "actrail-stdio-stdin-e2e");
        assert_eq!(config.stdio_continue_message, "actrail-stdio-continue-e2e");
        assert!(config.mmap.is_some());
    }
}

//! Command-line parser implementation for the eBPF probe tool.

use config_core::daemon::{
    ApplicationProtocolConfig, EnforcementConfig, PayloadSocketConfig, PayloadStdioConfig,
    PayloadTlsConfig, ResourceMetricsConfig,
};
use payload_capability::DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES;

use crate::probe_config;

use super::{LiveVerificationConfig, ProbeCommand, WorkloadConfig};

#[path = "parse/values.rs"]
mod values;

use values::{
    config_only, optional_bool, optional_mmap_config, parse_flags, required, required_bool,
    required_disabled_or_path, required_disabled_or_u64, required_enforcement_backend,
    required_enforcement_decision, required_enforcement_mark_strategy, required_enforcement_scope,
    required_memlock_rlimit, required_octal_u32, required_path, required_payload_redaction_policy,
    required_payload_socket_capture_backend, required_payload_socket_seccomp_syscalls,
    required_payload_stdio_storage_mode, required_payload_tls_capture_backend,
    required_payload_tls_library, required_payload_tls_library_path, required_payload_tls_resolver,
    required_payload_tls_seccomp_syscalls, required_payload_tls_source,
    required_payload_tls_sync_runtime_library_path, required_sse_data_policy, required_u32,
    required_u64,
};

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ProbeCommand, String> {
    let mut args = args.into_iter();
    let command = args.next().ok_or_else(|| "missing command".to_string())?;
    let flags = parse_flags(args)?;
    if flags.contains_key("--config") && !config_only(&flags) {
        return Err("--config cannot be combined with individual flags".to_string());
    }
    match command.as_str() {
        "verify-live" if config_only(&flags) => {
            let config_path = required_path(&flags, "--config")?;
            Ok(ProbeCommand::VerifyLive(
                probe_config::load_live_verification_config(&config_path)?,
            ))
        }
        "verify-live" => Ok(ProbeCommand::VerifyLive(LiveVerificationConfig {
            source_config_path: None,
            storage_path: required_path(&flags, "--storage-path")?,
            profile_name: required(&flags, "--profile-name")?,
            trace_name: required(&flags, "--trace-name")?,
            request_id_start: required_u64(&flags, "--request-id-start")?,
            memlock_rlimit: required_memlock_rlimit(&flags, "--memlock-rlimit")?,
            tracked_process_max_entries: required_u32(&flags, "--tracked-process-max-entries")?,
            pending_operation_max_entries: required_u32(&flags, "--pending-operation-max-entries")?,
            suppressed_fd_max_entries: required_u32(&flags, "--suppressed-fd-max-entries")?,
            suppressed_fd_index_slots_per_process: required_u32(
                &flags,
                "--suppressed-fd-index-slots-per-process",
            )?,
            event_ring_buffer_max_bytes: required_u32(&flags, "--event-ring-buffer-max-bytes")?,
            file_path_capture_enabled: required_bool(&flags, "--file-path-capture-enabled")?,
            file_path_max_bytes: required_u32(&flags, "--file-path-max-bytes")?,
            payload_tls: PayloadTlsConfig {
                enabled: required_bool(&flags, "--payload-tls-enabled")?,
                capture_backend: required_payload_tls_capture_backend(
                    &flags,
                    "--payload-tls-capture-backend",
                )?,
                source: required_payload_tls_source(&flags, "--payload-tls-source")?,
                resolver: required_payload_tls_resolver(&flags, "--payload-tls-resolver")?,
                library: required_payload_tls_library(&flags, "--payload-tls-library")?,
                library_path: required_payload_tls_library_path(
                    &flags,
                    "--payload-tls-library-path",
                )?,
                binary_path: required_disabled_or_path(&flags, "--payload-tls-binary-path")?,
                pattern_path: required_disabled_or_path(&flags, "--payload-tls-pattern-path")?,
                max_segment_bytes: required_u32(&flags, "--payload-tls-max-segment-bytes")?,
                max_operation_bytes: required_u32(&flags, "--payload-tls-max-operation-bytes")?,
                ring_buffer_bytes: required_u32(&flags, "--payload-tls-ring-buffer-bytes")?,
                pending_operation_max_entries: required_u32(
                    &flags,
                    "--payload-tls-pending-operation-max-entries",
                )?,
                seccomp_syscalls: required_payload_tls_seccomp_syscalls(
                    &flags,
                    "--payload-tls-seccomp-syscalls",
                )?,
                diagnostics_enabled: optional_bool(
                    &flags,
                    "--payload-tls-diagnostics-enabled",
                    false,
                )?,
                retention_max_bytes_per_trace: required_u64(
                    &flags,
                    "--payload-tls-retention-max-bytes-per-trace",
                )?,
                redaction_policy: required_payload_redaction_policy(
                    &flags,
                    "--payload-tls-redaction-policy",
                )?,
                sync_runtime_library_path: required_payload_tls_sync_runtime_library_path(
                    &flags,
                    "--payload-tls-sync-runtime-library-path",
                )?,
                sync_event_socket_path: required_path(
                    &flags,
                    "--payload-tls-sync-event-socket-path",
                )?,
                sync_socket_mode: required_octal_u32(
                    &flags,
                    "--payload-tls-sync-socket-mode-octal",
                )?,
                sync_match_limit: required_u32(&flags, "--payload-tls-sync-match-limit")?,
                sync_flow_control_enabled: true,
                sync_flow_sniff_bytes: 65536,
                sync_flow_max_header_bytes: 16384,
                sync_flow_large_transfer_bytes: 1048576,
                sync_flow_unknown_stream_bytes: DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES,
                sync_flow_h2_data_probe_bytes: 65536,
                java_agent_enabled: optional_bool(
                    &flags,
                    "--payload-tls-java-agent-enabled",
                    false,
                )?,
            },
            payload_stdio: PayloadStdioConfig {
                enabled: required_bool(&flags, "--payload-stdio-enabled")?,
                capture_stdin: required_bool(&flags, "--payload-stdio-capture-stdin")?,
                capture_stdout: required_bool(&flags, "--payload-stdio-capture-stdout")?,
                capture_stderr: required_bool(&flags, "--payload-stdio-capture-stderr")?,
                stdin_storage_mode: required_payload_stdio_storage_mode(
                    &flags,
                    "--payload-stdio-stdin-storage-mode",
                )?,
                stdout_storage_mode: required_payload_stdio_storage_mode(
                    &flags,
                    "--payload-stdio-stdout-storage-mode",
                )?,
                stderr_storage_mode: required_payload_stdio_storage_mode(
                    &flags,
                    "--payload-stdio-stderr-storage-mode",
                )?,
                max_segment_bytes: required_u32(&flags, "--payload-stdio-max-segment-bytes")?,
                ring_buffer_bytes: required_u32(&flags, "--payload-stdio-ring-buffer-bytes")?,
                pending_operation_max_entries: required_u32(
                    &flags,
                    "--payload-stdio-pending-operation-max-entries",
                )?,
                stream_state_max_entries: required_u32(
                    &flags,
                    "--payload-stdio-stream-state-max-entries",
                )?,
                retention_max_bytes_per_trace: required_u64(
                    &flags,
                    "--payload-stdio-retention-max-bytes-per-trace",
                )?,
                redaction_policy: required_payload_redaction_policy(
                    &flags,
                    "--payload-stdio-redaction-policy",
                )?,
            },
            payload_socket: PayloadSocketConfig {
                enabled: required_bool(&flags, "--payload-socket-enabled")?,
                capture_backend: required_payload_socket_capture_backend(
                    &flags,
                    "--payload-socket-capture-backend",
                )?,
                max_segment_bytes: required_u32(&flags, "--payload-socket-max-segment-bytes")?,
                max_operation_bytes: required_u32(&flags, "--payload-socket-max-operation-bytes")?,
                ring_buffer_bytes: required_u32(&flags, "--payload-socket-ring-buffer-bytes")?,
                pending_operation_max_entries: required_u32(
                    &flags,
                    "--payload-socket-pending-operation-max-entries",
                )?,
                stream_state_max_entries: required_u32(
                    &flags,
                    "--payload-socket-stream-state-max-entries",
                )?,
                retention_max_bytes_per_trace: required_u64(
                    &flags,
                    "--payload-socket-retention-max-bytes-per-trace",
                )?,
                redaction_policy: required_payload_redaction_policy(
                    &flags,
                    "--payload-socket-redaction-policy",
                )?,
                http_sniff_max_bytes: required_u64(
                    &flags,
                    "--payload-socket-http-sniff-max-bytes",
                )?,
                seccomp_syscalls: required_payload_socket_seccomp_syscalls(
                    &flags,
                    "--payload-socket-seccomp-syscalls",
                )?,
            },
            application_protocol: ApplicationProtocolConfig {
                enabled: required_bool(&flags, "--application-protocol-enabled")?,
                http1_enabled: required_bool(&flags, "--application-protocol-http1-enabled")?,
                http2_enabled: required_bool(&flags, "--application-protocol-http2-enabled")?,
                capture_host: required_bool(&flags, "--application-http-capture-host")?,
                sse_enabled: required_bool(&flags, "--application-http-sse-enabled")?,
                sse_data_policy: required_sse_data_policy(
                    &flags,
                    "--application-http-sse-data-policy",
                )?,
                sse_max_buffer_bytes: required_u64(
                    &flags,
                    "--application-http-sse-max-buffer-bytes",
                )?,
                sse_max_data_bytes: required_u64(&flags, "--application-http-sse-max-data-bytes")?,
                http2_max_frame_bytes: required_u64(&flags, "--application-http2-max-frame-bytes")?,
                http2_max_connection_buffer_bytes: required_u64(
                    &flags,
                    "--application-http2-max-connection-buffer-bytes",
                )?,
                http2_emit_data_preview: required_bool(
                    &flags,
                    "--application-http2-emit-data-preview",
                )?,
                http2_max_data_preview_bytes: required_u64(
                    &flags,
                    "--application-http2-max-data-preview-bytes",
                )?,
            },
            resource_metrics: ResourceMetricsConfig {
                enabled: required_bool(&flags, "--resource-metrics-enabled")?,
                interval_ms: required_u64(&flags, "--resource-metrics-interval-ms")?,
                include_children: required_bool(&flags, "--resource-metrics-include-children")?,
                include_system: required_bool(&flags, "--resource-metrics-include-system")?,
                cpu_alert_percent_millis: required_disabled_or_u64(
                    &flags,
                    "--resource-metrics-cpu-alert-percent-millis",
                )?,
                memory_alert_rss_kb: required_disabled_or_u64(
                    &flags,
                    "--resource-metrics-memory-alert-rss-kb",
                )?,
            },
            enforcement: EnforcementConfig {
                enabled: required_bool(&flags, "--enforcement-enabled")?,
                backend: required_enforcement_backend(&flags, "--enforcement-backend")?,
                scope: required_enforcement_scope(&flags, "--enforcement-scope")?,
                rules_path: required_path(&flags, "--enforcement-rules-path")?,
                builtin_rules: Vec::new(),
                default_decision: required_enforcement_decision(
                    &flags,
                    "--enforcement-default-decision",
                )?,
                mark_strategy: required_enforcement_mark_strategy(
                    &flags,
                    "--enforcement-mark-strategy",
                )?,
                audit_enabled: required_bool(&flags, "--enforcement-audit-enabled")?,
                event_buffer_bytes: required_u32(&flags, "--enforcement-event-buffer-bytes")?,
            },
            process_signal_number: required_u32(&flags, "--process-signal-number")?,
            exec_path: required_path(&flags, "--exec-path")?,
            listen_addr: required(&flags, "--listen-addr")?,
            client_message: required(&flags, "--client-message")?,
            server_message: required(&flags, "--server-message")?,
            stdio_stdin_message: required(&flags, "--stdio-stdin-message")?,
            stdio_continue_message: required(&flags, "--stdio-continue-message")?,
            stdio_stdout_message: required(&flags, "--stdio-stdout-message")?,
            stdio_stderr_message: required(&flags, "--stdio-stderr-message")?,
            pipe_message: required(&flags, "--pipe-message")?,
            fifo_path: required_path(&flags, "--fifo-path")?,
            fifo_mode: required_octal_u32(&flags, "--fifo-mode")?,
            file_path: required_path(&flags, "--file-path")?,
            file_message: required(&flags, "--file-message")?,
            mmap: optional_mmap_config(&flags)?,
            mkdir_path: required_path(&flags, "--mkdir-path")?,
            rmdir_path: required_path(&flags, "--rmdir-path")?,
            rename_source_path: required_path(&flags, "--rename-source-path")?,
            rename_target_path: required_path(&flags, "--rename-target-path")?,
            unlink_path: required_path(&flags, "--unlink-path")?,
            truncate_path: required_path(&flags, "--truncate-path")?,
            unix_message: required(&flags, "--unix-message")?,
            directory_mode: required_octal_u32(&flags, "--directory-mode")?,
            provider_rules_path: required_path(&flags, "--provider-rules-path")?,
            provider_unknown_provider_label: required(&flags, "--provider-unknown-provider-label")?,
            provider_expected_provider: required(&flags, "--provider-expected-provider")?,
        })),
        "workload" if config_only(&flags) => {
            let config_path = required_path(&flags, "--config")?;
            Ok(ProbeCommand::Workload(probe_config::load_workload_config(
                &config_path,
            )?))
        }
        "workload" => Ok(ProbeCommand::Workload(WorkloadConfig {
            exec_path: required_path(&flags, "--exec-path")?,
            listen_addr: required(&flags, "--listen-addr")?,
            client_message: required(&flags, "--client-message")?,
            server_message: required(&flags, "--server-message")?,
            stdio_stdin_message: required(&flags, "--stdio-stdin-message")?,
            stdio_continue_message: required(&flags, "--stdio-continue-message")?,
            stdio_stdout_message: required(&flags, "--stdio-stdout-message")?,
            stdio_stderr_message: required(&flags, "--stdio-stderr-message")?,
            process_signal_number: required_u32(&flags, "--process-signal-number")?,
            pipe_message: required(&flags, "--pipe-message")?,
            fifo_path: required_path(&flags, "--fifo-path")?,
            fifo_mode: required_octal_u32(&flags, "--fifo-mode")?,
            file_path: required_path(&flags, "--file-path")?,
            file_message: required(&flags, "--file-message")?,
            mmap: optional_mmap_config(&flags)?,
            mkdir_path: required_path(&flags, "--mkdir-path")?,
            rmdir_path: required_path(&flags, "--rmdir-path")?,
            rename_source_path: required_path(&flags, "--rename-source-path")?,
            rename_target_path: required_path(&flags, "--rename-target-path")?,
            unlink_path: required_path(&flags, "--unlink-path")?,
            truncate_path: required_path(&flags, "--truncate-path")?,
            unix_message: required(&flags, "--unix-message")?,
            directory_mode: required_octal_u32(&flags, "--directory-mode")?,
        })),
        other => Err(format!("unknown command {other}")),
    }
}

pub fn workload_from_live_config(config: &LiveVerificationConfig) -> WorkloadConfig {
    WorkloadConfig {
        exec_path: config.exec_path.clone(),
        listen_addr: config.listen_addr.clone(),
        client_message: config.client_message.clone(),
        server_message: config.server_message.clone(),
        stdio_stdin_message: config.stdio_stdin_message.clone(),
        stdio_continue_message: config.stdio_continue_message.clone(),
        stdio_stdout_message: config.stdio_stdout_message.clone(),
        stdio_stderr_message: config.stdio_stderr_message.clone(),
        process_signal_number: config.process_signal_number,
        pipe_message: config.pipe_message.clone(),
        fifo_path: config.fifo_path.clone(),
        fifo_mode: config.fifo_mode,
        file_path: config.file_path.clone(),
        file_message: config.file_message.clone(),
        mmap: config.mmap.clone(),
        mkdir_path: config.mkdir_path.clone(),
        rmdir_path: config.rmdir_path.clone(),
        rename_source_path: config.rename_source_path.clone(),
        rename_target_path: config.rename_target_path.clone(),
        unlink_path: config.unlink_path.clone(),
        truncate_path: config.truncate_path.clone(),
        unix_message: config.unix_message.clone(),
        directory_mode: config.directory_mode,
    }
}

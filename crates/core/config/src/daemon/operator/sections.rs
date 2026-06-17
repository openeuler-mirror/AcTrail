//! Operator config section factories.

use super::super::values::{ConfigNode, ConfigValues};
use super::super::{
    AgentInvocationConfig, ApplicationProtocolConfig, DisabledOrPath, EnforcementConfig,
    FileBulkReadObservationConfig, FileObservationConfig, FileTtyObservationConfig,
    FsEnumerateObservationConfig, L0LlmCallRetention, L1SseRetention, L2HttpRetention,
    L3Http2FrameRetention, L4PayloadRetention, PayloadRedactionPolicy, PayloadSocketCaptureBackend,
    PayloadSocketConfig, PayloadSocketSeccompSyscall, PayloadStdioConfig, PayloadStdioStorageMode,
    PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibrary, PayloadTlsLibraryPath,
    PayloadTlsResolver, PayloadTlsSeccompSyscall, PayloadTlsSource,
    PayloadTlsSyncRuntimeLibraryPath, ProcessSeccompConfig, ProcessSeccompSyscall,
    ResourceMetricsConfig, SeccompNotifyConfig, SemanticRetentionConfig,
};
use crate::export::ExportConfig;
use crate::provider_rules::ProviderRuleSetConfig;

pub(super) fn application_protocol_config(
    protocol: ConfigNode,
    http: ConfigNode,
    http2: ConfigNode,
) -> Result<ApplicationProtocolConfig, String> {
    Ok(ApplicationProtocolConfig {
        enabled: protocol.required_bool("enabled")?,
        http1_enabled: protocol.required_bool("http1_enabled")?,
        http2_enabled: protocol.required_bool("http2_enabled")?,
        capture_host: http.required_bool("capture_host")?,
        sse_enabled: http.required_bool("sse_enabled")?,
        sse_data_policy: http.required_parsed("sse_data_policy")?,
        sse_max_buffer_bytes: http.required_positive_u64("sse_max_buffer_bytes")?,
        sse_max_data_bytes: http.required_positive_u64("sse_max_data_bytes")?,
        http2_max_frame_bytes: http2.required_positive_u64("max_frame_bytes")?,
        http2_max_connection_buffer_bytes: http2
            .required_positive_u64("max_connection_buffer_bytes")?,
        http2_emit_data_preview: http2.required_bool("emit_data_preview")?,
        http2_max_data_preview_bytes: http2.required_positive_u64("max_data_preview_bytes")?,
    })
}

pub(super) fn resource_metrics_config(node: ConfigNode) -> Result<ResourceMetricsConfig, String> {
    Ok(ResourceMetricsConfig {
        enabled: node.required_bool("enabled")?,
        interval_ms: node.required_positive_u64("interval_ms")?,
        include_children: node.required_bool("include_children")?,
        include_system: node.required_bool("include_system")?,
        cpu_alert_percent_millis: node
            .required_disabled_or_positive_u64("cpu_alert_percent_millis")?,
        memory_alert_rss_kb: node.required_disabled_or_positive_u64("memory_alert_rss_kb")?,
    })
}

pub(super) fn enforcement_config(node: ConfigNode) -> Result<EnforcementConfig, String> {
    Ok(EnforcementConfig {
        enabled: node.required_bool("enabled")?,
        backend: node.required_parsed("backend")?,
        scope: node.required_parsed("scope")?,
        rules_path: node.required_path_buf("rules_path")?,
        default_decision: node.required_parsed("default_decision")?,
        mark_strategy: node.required_parsed("mark_strategy")?,
        audit_enabled: node.required_bool("audit_enabled")?,
        event_buffer_bytes: node.required_positive_u32("event_buffer_bytes")?,
    })
}

pub(super) fn export_config(
    root: &ConfigValues,
    export: ConfigNode,
) -> Result<ExportConfig, String> {
    Ok(ExportConfig {
        graph_schema_version: root.required("graph_schema_version")?,
        allow_active_trace_snapshot: root.required_bool("allow_active_trace_snapshot")?,
        output_directory: export.required_path_buf("directory")?,
        payload_bytes_enabled: export.required_bool("payload_bytes_enabled")?,
        payload_text_enabled: export.required_bool("payload_text_enabled")?,
    })
}

pub(super) fn provider_rule_set_config(
    node: ConfigNode,
) -> Result<Option<ProviderRuleSetConfig>, String> {
    let enabled = node.required_bool("rules_enabled")?;
    let rules_path = node.required_path_buf("rules_path")?;
    let unknown_provider_label = node.required("unknown_provider_label")?;
    Ok(enabled.then(|| ProviderRuleSetConfig {
        rules_path,
        unknown_provider_label,
    }))
}

pub(super) fn payload_tls_config(node: ConfigNode) -> Result<PayloadTlsConfig, String> {
    Ok(PayloadTlsConfig {
        enabled: node.required_bool("enabled")?,
        capture_backend: node.required_parsed::<PayloadTlsCaptureBackend>("capture_backend")?,
        source: node.required_parsed::<PayloadTlsSource>("source")?,
        resolver: node.required_parsed::<PayloadTlsResolver>("resolver")?,
        library: node.required_parsed::<PayloadTlsLibrary>("library")?,
        library_path: node.required_parsed::<PayloadTlsLibraryPath>("library_path")?,
        binary_path: node.required_parsed::<DisabledOrPath>("binary_path")?,
        pattern_path: node.required_parsed::<DisabledOrPath>("pattern_path")?,
        max_segment_bytes: node.required_positive_u32("max_segment_bytes")?,
        max_operation_bytes: node.required_positive_u32("max_operation_bytes")?,
        ring_buffer_bytes: node.required_positive_u32("ring_buffer_bytes")?,
        pending_operation_max_entries: node
            .required_positive_u32("pending_operation_max_entries")?,
        seccomp_syscalls: node.repeated_parsed::<PayloadTlsSeccompSyscall>("seccomp_syscall")?,
        diagnostics_enabled: node.optional_bool("diagnostics_enabled", false)?,
        retention_max_bytes_per_trace: node
            .required_positive_u64("retention_max_bytes_per_trace")?,
        redaction_policy: node.required_parsed::<PayloadRedactionPolicy>("redaction_policy")?,
        sync_runtime_library_path: node
            .required_parsed::<PayloadTlsSyncRuntimeLibraryPath>("sync_runtime_library_path")?,
        sync_event_socket_path: node.required_path_buf("sync_event_socket_path")?,
        sync_socket_mode: node.required_octal("sync_socket_mode_octal")?,
        sync_match_limit: node.required_positive_u32("sync_match_limit")?,
        java_agent_enabled: node.optional_bool("java_agent_enabled", false)?,
    })
}

pub(super) fn seccomp_notify_config(node: ConfigNode) -> Result<SeccompNotifyConfig, String> {
    Ok(SeccompNotifyConfig {
        enabled: node.required_bool("enabled")?,
        reserved_listener_fd: node.required_positive_u32("reserved_listener_fd")?,
    })
}

pub(super) fn process_seccomp_config(node: ConfigNode) -> Result<ProcessSeccompConfig, String> {
    Ok(ProcessSeccompConfig {
        enabled: node.required_bool("enabled")?,
        syscalls: node.repeated_parsed::<ProcessSeccompSyscall>("syscall")?,
        max_args: node.required_positive_u32("max_args")?,
        max_arg_bytes: node.required_positive_u32("max_arg_bytes")?,
        pending_max_entries: node.required_positive_u32("pending_max_entries")?,
    })
}

pub(super) fn agent_invocation_config(node: ConfigNode) -> Result<AgentInvocationConfig, String> {
    Ok(AgentInvocationConfig {
        enabled: node.required_bool("enabled")?,
        commands: node.repeated_optional("command").cloned().collect(),
    })
}

pub(super) fn semantic_retention_config(
    node: ConfigNode,
) -> Result<SemanticRetentionConfig, String> {
    let defaults = SemanticRetentionConfig::default();
    let l0 = L0LlmCallRetention::default();
    let l1 = L1SseRetention::default();
    let l2 = L2HttpRetention::default();
    let l3 = L3Http2FrameRetention::default();
    let l4 = L4PayloadRetention::default();
    Ok(SemanticRetentionConfig {
        content_owner: node.optional_parsed("content_owner", defaults.content_owner)?,
        l0_llm_call: L0LlmCallRetention {
            enabled: node.optional_bool("L0_llm_call_enabled", l0.enabled)?,
            request_content: node
                .optional_parsed("L0_llm_call_request_content", l0.request_content)?,
            response_content: node
                .optional_parsed("L0_llm_call_response_content", l0.response_content)?,
            tool_calls: node.optional_parsed("L0_llm_call_tool_calls", l0.tool_calls)?,
            usage: node.optional_parsed("L0_llm_call_usage", l0.usage)?,
        },
        l1_sse: L1SseRetention {
            enabled: node.optional_bool("L1_sse_enabled", l1.enabled)?,
            stream_summary: node.optional_bool("L1_sse_stream_summary", l1.stream_summary)?,
            event_content: node.optional_parsed("L1_sse_event_content", l1.event_content)?,
        },
        l2_http: L2HttpRetention {
            enabled: node.optional_bool("L2_http_enabled", l2.enabled)?,
            message_summary: node.optional_bool("L2_http_message_summary", l2.message_summary)?,
            headers: node.optional_parsed("L2_http_headers", l2.headers)?,
            body_content: node.optional_parsed("L2_http_body_content", l2.body_content)?,
        },
        l3_http2_frame: L3Http2FrameRetention {
            enabled: node.optional_bool("L3_http2_frame_enabled", l3.enabled)?,
            frame_summary: node.optional_bool("L3_http2_frame_frame_summary", l3.frame_summary)?,
            data_content: node.optional_parsed("L3_http2_frame_data_content", l3.data_content)?,
        },
        l4_payload: L4PayloadRetention {
            enabled: node.optional_bool("L4_payload_enabled", l4.enabled)?,
            stats: node.optional_bool("L4_payload_stats", l4.stats)?,
            body_content: node.optional_parsed("L4_payload_body_content", l4.body_content)?,
        },
    })
}

pub(super) fn file_observation_config(node: ConfigNode) -> Result<FileObservationConfig, String> {
    let defaults = FileObservationConfig::default();
    let tty = FileTtyObservationConfig::default();
    let bulk_read = FileBulkReadObservationConfig::default();
    let enumerate = FsEnumerateObservationConfig::default();
    let config = FileObservationConfig {
        enabled: node.optional_bool("enabled", defaults.enabled)?,
        metadata_retention: node
            .optional_parsed("metadata_retention", defaults.metadata_retention)?,
        tty: FileTtyObservationConfig {
            enabled: node.optional_bool("tty_enabled", tty.enabled)?,
            paths: optional_repeated_non_empty(&node, "tty_path", tty.paths)?,
            operations: optional_repeated_non_empty(&node, "tty_operation", tty.operations)?,
            raw_event_retention: node
                .optional_parsed("tty_raw_event_retention", tty.raw_event_retention)?,
        },
        bulk_read: FileBulkReadObservationConfig {
            enabled: node.optional_bool("bulk_read_enabled", bulk_read.enabled)?,
            mode: node.optional_parsed("bulk_read_mode", bulk_read.mode)?,
            raw_event_retention: node.optional_parsed(
                "bulk_read_raw_event_retention",
                bulk_read.raw_event_retention,
            )?,
            min_unique_paths: node
                .optional_positive_u32("bulk_read_min_unique_paths", bulk_read.min_unique_paths)?,
            max_paths_per_set: node.optional_positive_u32(
                "bulk_read_max_paths_per_set",
                bulk_read.max_paths_per_set,
            )?,
            path_set_chunk_max_paths: node.optional_positive_u32(
                "bulk_read_path_set_chunk_max_paths",
                bulk_read.path_set_chunk_max_paths,
            )?,
        },
        enumerate: FsEnumerateObservationConfig {
            enabled: node.optional_bool("enumerate_enabled", enumerate.enabled)?,
            raw_event_retention: node.optional_parsed(
                "enumerate_raw_event_retention",
                enumerate.raw_event_retention,
            )?,
            min_unique_paths: node
                .optional_positive_u32("enumerate_min_unique_paths", enumerate.min_unique_paths)?,
            max_paths_per_set: node.optional_positive_u32(
                "enumerate_max_paths_per_set",
                enumerate.max_paths_per_set,
            )?,
            path_set_chunk_max_paths: node.optional_positive_u32(
                "enumerate_path_set_chunk_max_paths",
                enumerate.path_set_chunk_max_paths,
            )?,
        },
    };
    if config.bulk_read.max_paths_per_set < config.bulk_read.min_unique_paths {
        return Err(
            "file_observation_bulk_read_max_paths_per_set must be >= file_observation_bulk_read_min_unique_paths"
                .to_string(),
        );
    }
    if config.enumerate.max_paths_per_set < config.enumerate.min_unique_paths {
        return Err(
            "file_observation_enumerate_max_paths_per_set must be >= file_observation_enumerate_min_unique_paths"
                .to_string(),
        );
    }
    Ok(config)
}

pub(super) fn payload_stdio_config(node: ConfigNode) -> Result<PayloadStdioConfig, String> {
    Ok(PayloadStdioConfig {
        enabled: node.required_bool("enabled")?,
        capture_stdin: node.required_bool("capture_stdin")?,
        capture_stdout: node.required_bool("capture_stdout")?,
        capture_stderr: node.required_bool("capture_stderr")?,
        stdin_storage_mode: node
            .required_parsed::<PayloadStdioStorageMode>("stdin_storage_mode")?,
        stdout_storage_mode: node
            .required_parsed::<PayloadStdioStorageMode>("stdout_storage_mode")?,
        stderr_storage_mode: node
            .required_parsed::<PayloadStdioStorageMode>("stderr_storage_mode")?,
        max_segment_bytes: node.required_positive_u32("max_segment_bytes")?,
        ring_buffer_bytes: node.required_positive_u32("ring_buffer_bytes")?,
        pending_operation_max_entries: node
            .required_positive_u32("pending_operation_max_entries")?,
        stream_state_max_entries: node.required_positive_u32("stream_state_max_entries")?,
        retention_max_bytes_per_trace: node
            .required_positive_u64("retention_max_bytes_per_trace")?,
        redaction_policy: node.required_parsed::<PayloadRedactionPolicy>("redaction_policy")?,
    })
}

fn optional_repeated_non_empty(
    node: &ConfigNode,
    key: &'static str,
    default: Vec<String>,
) -> Result<Vec<String>, String> {
    let values = node.repeated_optional(key).cloned().collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(default);
    }
    if values.iter().any(|value| value.is_empty()) {
        return Err(format!(
            "config key file_observation_{key} must not be empty"
        ));
    }
    Ok(values)
}

pub(super) fn payload_socket_config(node: ConfigNode) -> Result<PayloadSocketConfig, String> {
    Ok(PayloadSocketConfig {
        enabled: node.required_bool("enabled")?,
        capture_backend: node.required_parsed::<PayloadSocketCaptureBackend>("capture_backend")?,
        max_segment_bytes: node.required_positive_u32("max_segment_bytes")?,
        max_operation_bytes: node.required_positive_u32("max_operation_bytes")?,
        ring_buffer_bytes: node.required_positive_u32("ring_buffer_bytes")?,
        pending_operation_max_entries: node
            .required_positive_u32("pending_operation_max_entries")?,
        stream_state_max_entries: node.required_positive_u32("stream_state_max_entries")?,
        retention_max_bytes_per_trace: node
            .required_positive_u64("retention_max_bytes_per_trace")?,
        redaction_policy: node.required_parsed::<PayloadRedactionPolicy>("redaction_policy")?,
        http_sniff_max_bytes: node.required_positive_u64("http_sniff_max_bytes")?,
        seccomp_syscalls: node.repeated_parsed::<PayloadSocketSeccompSyscall>("seccomp_syscall")?,
    })
}

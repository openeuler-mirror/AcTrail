//! Operator config section factories.

use super::super::values::{ConfigNode, ConfigValues};
use super::super::{
    AgentInvocationConfig, ApplicationProtocolConfig, DisabledOrPath, EnforcementConfig,
    PayloadRedactionPolicy, PayloadSocketCaptureBackend, PayloadSocketConfig,
    PayloadSocketSeccompSyscall, PayloadStdioConfig, PayloadStdioStorageMode,
    PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibrary, PayloadTlsLibraryPath,
    PayloadTlsResolver, PayloadTlsSeccompSyscall, PayloadTlsSource,
    PayloadTlsSyncRuntimeLibraryPath, ProcessSeccompConfig, ProcessSeccompSyscall,
    ResourceMetricsConfig, SeccompNotifyConfig,
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

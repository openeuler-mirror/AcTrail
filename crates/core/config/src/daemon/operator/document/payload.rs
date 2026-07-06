use super::*;

use crate::daemon::DEFAULT_MCP_PARSE_BUFFER_MAX_BYTES;
use payload_capability::DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PayloadDocument {
    pub tls: PayloadTlsDocument,
    pub stdio: PayloadStdioDocument,
    pub socket: PayloadSocketDocument,
    pub mcp: PayloadMcpDocument,
}

impl Default for PayloadDocument {
    fn default() -> Self {
        Self {
            tls: PayloadTlsDocument::default(),
            stdio: PayloadStdioDocument::default(),
            socket: PayloadSocketDocument::default(),
            mcp: PayloadMcpDocument::default(),
        }
    }
}

impl PayloadDocument {
    pub(super) fn from_config(config: &PayloadConfig) -> Self {
        Self {
            tls: PayloadTlsDocument::from_config(&config.tls),
            stdio: PayloadStdioDocument::from_config(&config.stdio),
            socket: PayloadSocketDocument::from_config(&config.socket),
            mcp: PayloadMcpDocument::from_config(&config.mcp),
        }
    }
}

impl PayloadDocument {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PayloadMcpDocument {
    pub parse_buffer_max_bytes: u64,
}

impl Default for PayloadMcpDocument {
    fn default() -> Self {
        Self {
            parse_buffer_max_bytes: DEFAULT_MCP_PARSE_BUFFER_MAX_BYTES,
        }
    }
}

impl PayloadMcpDocument {
    pub(super) fn from_config(config: &PayloadMcpConfig) -> Self {
        Self {
            parse_buffer_max_bytes: config.parse_buffer_max_bytes,
        }
    }

    pub(super) fn to_config(&self) -> Result<PayloadMcpConfig, String> {
        Ok(PayloadMcpConfig {
            parse_buffer_max_bytes: require_positive_u64(
                "payload.mcp.parse_buffer_max_bytes",
                self.parse_buffer_max_bytes,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PayloadTlsDocument {
    pub enabled: bool,
    pub capture_backend: String,
    pub source: String,
    pub resolver: String,
    pub library: String,
    pub library_path: String,
    pub binary_path: String,
    pub pattern_path: String,
    pub max_segment_bytes: u32,
    pub max_operation_bytes: u32,
    pub ring_buffer_bytes: u32,
    pub pending_operation_max_entries: u32,
    pub seccomp_syscalls: Vec<String>,
    pub diagnostics_enabled: bool,
    pub retention_max_bytes_per_trace: u64,
    pub redaction_policy: String,
    pub sync_runtime_library_path: String,
    pub sync_event_socket_path: String,
    pub sync_socket_mode_octal: String,
    pub sync_match_limit: u32,
    pub sync_flow_control_enabled: bool,
    pub sync_flow_sniff_bytes: u32,
    pub sync_flow_max_header_bytes: u32,
    pub sync_flow_large_transfer_bytes: u64,
    pub sync_flow_unknown_stream_bytes: u64,
    pub sync_flow_h2_data_probe_bytes: u64,
    pub java_agent_enabled: bool,
}

impl Default for PayloadTlsDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_backend: "tls-sync".to_string(),
            source: "auto".to_string(),
            resolver: "auto".to_string(),
            library: "auto".to_string(),
            library_path: "auto".to_string(),
            binary_path: "disabled".to_string(),
            pattern_path: "disabled".to_string(),
            max_segment_bytes: 4095,
            max_operation_bytes: 16777216,
            ring_buffer_bytes: 8388608,
            pending_operation_max_entries: 8192,
            seccomp_syscalls: ["write", "writev", "sendto", "sendmsg"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            diagnostics_enabled: true,
            retention_max_bytes_per_trace: 104857600,
            redaction_policy: "disabled".to_string(),
            sync_runtime_library_path: "auto".to_string(),
            sync_event_socket_path: "/run/actrail/tls-sync.sock".to_string(),
            sync_socket_mode_octal: "660".to_string(),
            sync_match_limit: 8,
            sync_flow_control_enabled: true,
            sync_flow_sniff_bytes: 65536,
            sync_flow_max_header_bytes: 16384,
            sync_flow_large_transfer_bytes: 1048576,
            sync_flow_unknown_stream_bytes: DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES,
            sync_flow_h2_data_probe_bytes: 65536,
            java_agent_enabled: false,
        }
    }
}

impl PayloadTlsDocument {
    pub(super) fn from_config(config: &PayloadTlsConfig) -> Self {
        Self {
            enabled: config.enabled,
            capture_backend: payload_tls_capture_backend_as_str(config.capture_backend).to_string(),
            source: payload_tls_source_as_str(config.source).to_string(),
            resolver: payload_tls_resolver_as_str(config.resolver).to_string(),
            library: payload_tls_library_as_str(config.library).to_string(),
            library_path: payload_tls_library_path_as_string(&config.library_path),
            binary_path: disabled_or_path_as_string(&config.binary_path),
            pattern_path: disabled_or_path_as_string(&config.pattern_path),
            max_segment_bytes: config.max_segment_bytes,
            max_operation_bytes: config.max_operation_bytes,
            ring_buffer_bytes: config.ring_buffer_bytes,
            pending_operation_max_entries: config.pending_operation_max_entries,
            seccomp_syscalls: config
                .seccomp_syscalls
                .iter()
                .map(payload_tls_seccomp_syscall_as_str)
                .map(str::to_string)
                .collect(),
            diagnostics_enabled: config.diagnostics_enabled,
            retention_max_bytes_per_trace: config.retention_max_bytes_per_trace,
            redaction_policy: payload_redaction_policy_as_str(config.redaction_policy).to_string(),
            sync_runtime_library_path: payload_tls_sync_runtime_library_path_as_string(
                &config.sync_runtime_library_path,
            ),
            sync_event_socket_path: config.sync_event_socket_path.display().to_string(),
            sync_socket_mode_octal: format!("{:o}", config.sync_socket_mode),
            sync_match_limit: config.sync_match_limit,
            sync_flow_control_enabled: config.sync_flow_control_enabled,
            sync_flow_sniff_bytes: config.sync_flow_sniff_bytes,
            sync_flow_max_header_bytes: config.sync_flow_max_header_bytes,
            sync_flow_large_transfer_bytes: config.sync_flow_large_transfer_bytes,
            sync_flow_unknown_stream_bytes: config.sync_flow_unknown_stream_bytes,
            sync_flow_h2_data_probe_bytes: config.sync_flow_h2_data_probe_bytes,
            java_agent_enabled: config.java_agent_enabled,
        }
    }

    pub(super) fn to_config(&self) -> Result<PayloadTlsConfig, String> {
        Ok(PayloadTlsConfig {
            enabled: self.enabled,
            capture_backend: parse_value("payload.tls.capture_backend", &self.capture_backend)?,
            source: parse_value("payload.tls.source", &self.source)?,
            resolver: parse_value("payload.tls.resolver", &self.resolver)?,
            library: parse_value("payload.tls.library", &self.library)?,
            library_path: parse_value("payload.tls.library_path", &self.library_path)?,
            binary_path: parse_value("payload.tls.binary_path", &self.binary_path)?,
            pattern_path: parse_value("payload.tls.pattern_path", &self.pattern_path)?,
            max_segment_bytes: require_positive_u32(
                "payload.tls.max_segment_bytes",
                self.max_segment_bytes,
            )?,
            max_operation_bytes: require_positive_u32(
                "payload.tls.max_operation_bytes",
                self.max_operation_bytes,
            )?,
            ring_buffer_bytes: require_positive_u32(
                "payload.tls.ring_buffer_bytes",
                self.ring_buffer_bytes,
            )?,
            pending_operation_max_entries: require_positive_u32(
                "payload.tls.pending_operation_max_entries",
                self.pending_operation_max_entries,
            )?,
            seccomp_syscalls: parse_values("payload.tls.seccomp_syscalls", &self.seccomp_syscalls)?,
            diagnostics_enabled: self.diagnostics_enabled,
            retention_max_bytes_per_trace: require_positive_u64(
                "payload.tls.retention_max_bytes_per_trace",
                self.retention_max_bytes_per_trace,
            )?,
            redaction_policy: parse_value("payload.tls.redaction_policy", &self.redaction_policy)?,
            sync_runtime_library_path: parse_value(
                "payload.tls.sync_runtime_library_path",
                &self.sync_runtime_library_path,
            )?,
            sync_event_socket_path: PathBuf::from(&self.sync_event_socket_path),
            sync_socket_mode: parse_octal(
                "payload.tls.sync_socket_mode_octal",
                &self.sync_socket_mode_octal,
            )?,
            sync_match_limit: require_positive_u32(
                "payload.tls.sync_match_limit",
                self.sync_match_limit,
            )?,
            sync_flow_control_enabled: self.sync_flow_control_enabled,
            sync_flow_sniff_bytes: require_positive_u32(
                "payload.tls.sync_flow_sniff_bytes",
                self.sync_flow_sniff_bytes,
            )?,
            sync_flow_max_header_bytes: require_positive_u32(
                "payload.tls.sync_flow_max_header_bytes",
                self.sync_flow_max_header_bytes,
            )?,
            sync_flow_large_transfer_bytes: require_positive_u64(
                "payload.tls.sync_flow_large_transfer_bytes",
                self.sync_flow_large_transfer_bytes,
            )?,
            sync_flow_unknown_stream_bytes: require_positive_u64(
                "payload.tls.sync_flow_unknown_stream_bytes",
                self.sync_flow_unknown_stream_bytes,
            )?,
            sync_flow_h2_data_probe_bytes: require_positive_u64(
                "payload.tls.sync_flow_h2_data_probe_bytes",
                self.sync_flow_h2_data_probe_bytes,
            )?,
            java_agent_enabled: self.java_agent_enabled,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PayloadStdioDocument {
    pub enabled: bool,
    pub capture_stdin: bool,
    pub capture_stdout: bool,
    pub capture_stderr: bool,
    pub stdin_storage_mode: String,
    pub stdout_storage_mode: String,
    pub stderr_storage_mode: String,
    pub max_segment_bytes: u32,
    pub ring_buffer_bytes: u32,
    pub pending_operation_max_entries: u32,
    pub stream_state_max_entries: u32,
    pub retention_max_bytes_per_trace: u64,
    pub redaction_policy: String,
}

impl Default for PayloadStdioDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_stdin: true,
            capture_stdout: true,
            capture_stderr: true,
            stdin_storage_mode: "full".to_string(),
            stdout_storage_mode: "drop".to_string(),
            stderr_storage_mode: "metadata-only".to_string(),
            max_segment_bytes: 4095,
            ring_buffer_bytes: 4194304,
            pending_operation_max_entries: 8192,
            stream_state_max_entries: 8192,
            retention_max_bytes_per_trace: 104857600,
            redaction_policy: "disabled".to_string(),
        }
    }
}

impl PayloadStdioDocument {
    pub(super) fn from_config(config: &PayloadStdioConfig) -> Self {
        Self {
            enabled: config.enabled,
            capture_stdin: config.capture_stdin,
            capture_stdout: config.capture_stdout,
            capture_stderr: config.capture_stderr,
            stdin_storage_mode: payload_stdio_storage_mode_as_str(config.stdin_storage_mode)
                .to_string(),
            stdout_storage_mode: payload_stdio_storage_mode_as_str(config.stdout_storage_mode)
                .to_string(),
            stderr_storage_mode: payload_stdio_storage_mode_as_str(config.stderr_storage_mode)
                .to_string(),
            max_segment_bytes: config.max_segment_bytes,
            ring_buffer_bytes: config.ring_buffer_bytes,
            pending_operation_max_entries: config.pending_operation_max_entries,
            stream_state_max_entries: config.stream_state_max_entries,
            retention_max_bytes_per_trace: config.retention_max_bytes_per_trace,
            redaction_policy: payload_redaction_policy_as_str(config.redaction_policy).to_string(),
        }
    }

    pub(super) fn to_config(&self) -> Result<PayloadStdioConfig, String> {
        Ok(PayloadStdioConfig {
            enabled: self.enabled,
            capture_stdin: self.capture_stdin,
            capture_stdout: self.capture_stdout,
            capture_stderr: self.capture_stderr,
            stdin_storage_mode: parse_value(
                "payload.stdio.stdin_storage_mode",
                &self.stdin_storage_mode,
            )?,
            stdout_storage_mode: parse_value(
                "payload.stdio.stdout_storage_mode",
                &self.stdout_storage_mode,
            )?,
            stderr_storage_mode: parse_value(
                "payload.stdio.stderr_storage_mode",
                &self.stderr_storage_mode,
            )?,
            max_segment_bytes: require_positive_u32(
                "payload.stdio.max_segment_bytes",
                self.max_segment_bytes,
            )?,
            ring_buffer_bytes: require_positive_u32(
                "payload.stdio.ring_buffer_bytes",
                self.ring_buffer_bytes,
            )?,
            pending_operation_max_entries: require_positive_u32(
                "payload.stdio.pending_operation_max_entries",
                self.pending_operation_max_entries,
            )?,
            stream_state_max_entries: require_positive_u32(
                "payload.stdio.stream_state_max_entries",
                self.stream_state_max_entries,
            )?,
            retention_max_bytes_per_trace: require_positive_u64(
                "payload.stdio.retention_max_bytes_per_trace",
                self.retention_max_bytes_per_trace,
            )?,
            redaction_policy: parse_value(
                "payload.stdio.redaction_policy",
                &self.redaction_policy,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PayloadSocketDocument {
    pub enabled: bool,
    pub capture_backend: String,
    pub max_segment_bytes: u32,
    pub max_operation_bytes: u32,
    pub ring_buffer_bytes: u32,
    pub pending_operation_max_entries: u32,
    pub stream_state_max_entries: u32,
    pub retention_max_bytes_per_trace: u64,
    pub redaction_policy: String,
    pub http_sniff_max_bytes: u64,
    pub seccomp_syscalls: Vec<String>,
}

impl Default for PayloadSocketDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_backend: "bpf-copy-seccomp-fallback".to_string(),
            max_segment_bytes: 4095,
            max_operation_bytes: 4194304,
            ring_buffer_bytes: 8388608,
            pending_operation_max_entries: 8192,
            stream_state_max_entries: 8192,
            retention_max_bytes_per_trace: 104857600,
            redaction_policy: "disabled".to_string(),
            http_sniff_max_bytes: 65536,
            seccomp_syscalls: ["write", "writev", "sendto", "sendmsg"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }
}

impl PayloadSocketDocument {
    pub(super) fn from_config(config: &PayloadSocketConfig) -> Self {
        Self {
            enabled: config.enabled,
            capture_backend: payload_socket_capture_backend_as_str(config.capture_backend)
                .to_string(),
            max_segment_bytes: config.max_segment_bytes,
            max_operation_bytes: config.max_operation_bytes,
            ring_buffer_bytes: config.ring_buffer_bytes,
            pending_operation_max_entries: config.pending_operation_max_entries,
            stream_state_max_entries: config.stream_state_max_entries,
            retention_max_bytes_per_trace: config.retention_max_bytes_per_trace,
            redaction_policy: payload_redaction_policy_as_str(config.redaction_policy).to_string(),
            http_sniff_max_bytes: config.http_sniff_max_bytes,
            seccomp_syscalls: config
                .seccomp_syscalls
                .iter()
                .map(payload_socket_seccomp_syscall_as_str)
                .map(str::to_string)
                .collect(),
        }
    }

    pub(super) fn to_config(&self) -> Result<PayloadSocketConfig, String> {
        Ok(PayloadSocketConfig {
            enabled: self.enabled,
            capture_backend: parse_value("payload.socket.capture_backend", &self.capture_backend)?,
            max_segment_bytes: require_positive_u32(
                "payload.socket.max_segment_bytes",
                self.max_segment_bytes,
            )?,
            max_operation_bytes: require_positive_u32(
                "payload.socket.max_operation_bytes",
                self.max_operation_bytes,
            )?,
            ring_buffer_bytes: require_positive_u32(
                "payload.socket.ring_buffer_bytes",
                self.ring_buffer_bytes,
            )?,
            pending_operation_max_entries: require_positive_u32(
                "payload.socket.pending_operation_max_entries",
                self.pending_operation_max_entries,
            )?,
            stream_state_max_entries: require_positive_u32(
                "payload.socket.stream_state_max_entries",
                self.stream_state_max_entries,
            )?,
            retention_max_bytes_per_trace: require_positive_u64(
                "payload.socket.retention_max_bytes_per_trace",
                self.retention_max_bytes_per_trace,
            )?,
            redaction_policy: parse_value(
                "payload.socket.redaction_policy",
                &self.redaction_policy,
            )?,
            http_sniff_max_bytes: require_positive_u64(
                "payload.socket.http_sniff_max_bytes",
                self.http_sniff_max_bytes,
            )?,
            seccomp_syscalls: parse_values(
                "payload.socket.seccomp_syscalls",
                &self.seccomp_syscalls,
            )?,
        })
    }
}

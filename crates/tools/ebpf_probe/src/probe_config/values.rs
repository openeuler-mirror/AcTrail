//! Typed values parsed from eBPF probe config files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use config_core::daemon::{
    ApplicationProtocolConfig, EnforcementBackend, EnforcementConfig, EnforcementDecision,
    EnforcementMarkStrategy, EnforcementScope, MemlockRlimit, PayloadRedactionPolicy,
    PayloadSocketCaptureBackend, PayloadSocketConfig, PayloadSocketSeccompSyscall,
    PayloadStdioConfig, PayloadStdioStorageMode, PayloadTlsCaptureBackend, PayloadTlsConfig,
    PayloadTlsLibrary, PayloadTlsLibraryPath, PayloadTlsResolver, PayloadTlsSeccompSyscall,
    PayloadTlsSource, PayloadTlsSyncRuntimeLibraryPath, ResourceMetricsConfig, SseDataPolicy,
};

use crate::args::MmapWorkloadConfig;

pub(super) struct ConfigValues {
    values: BTreeMap<String, Vec<String>>,
}

impl ConfigValues {
    pub(super) fn parse(raw: &str, allowed_keys: BTreeSet<&'static str>) -> Result<Self, String> {
        let mut values = BTreeMap::<String, Vec<String>>::new();
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed
                .split_once('=')
                .ok_or_else(|| format!("invalid config line {}", line_index + 1))?;
            let key = key.trim().to_string();
            let value = unquote(value.trim())?;
            if !allowed_keys.contains(key.as_str()) {
                return Err(format!("unknown probe config key {key}"));
            }
            if !matches!(
                key.as_str(),
                "payload_tls_seccomp_syscall"
                    | "payload_socket_seccomp_syscall"
                    | "process_seccomp_syscall"
                    | "agent_invocation_command"
            ) && values.contains_key(&key)
            {
                return Err(format!("duplicate config key {key}"));
            }
            values.entry(key).or_default().push(value);
        }
        Ok(Self { values })
    }

    pub(super) fn required(&self, key: &'static str) -> Result<String, String> {
        let values = self
            .values
            .get(key)
            .filter(|values| values.len() == 1)
            .ok_or_else(|| format!("missing config key {key}"))?;
        values
            .first()
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {key}"))
    }

    pub(super) fn required_path(&self, key: &'static str) -> Result<PathBuf, String> {
        self.required(key).map(PathBuf::from)
    }

    pub(super) fn required_bool(&self, key: &'static str) -> Result<bool, String> {
        match self.required(key)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {key}: expected true or false, got {value}"
            )),
        }
    }

    fn optional_bool(&self, key: &'static str, default: bool) -> Result<bool, String> {
        let Some(values) = self.values.get(key) else {
            return Ok(default);
        };
        if values.len() != 1 {
            return Err(format!("config key {key} must appear once"));
        }
        match values
            .first()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {key}"))?
            .as_str()
        {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!(
                "invalid {key}: expected true or false, got {value}"
            )),
        }
    }

    fn optional_positive_u32(&self, key: &'static str, default: u32) -> Result<u32, String> {
        let Some(values) = self.values.get(key) else {
            return Ok(default);
        };
        if values.len() != 1 {
            return Err(format!("config key {key} must appear once"));
        }
        let value = values
            .first()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {key}"))?;
        let parsed = value
            .parse::<u32>()
            .map_err(|error| format!("invalid {key}: {error}"))?;
        if parsed == 0 {
            return Err(format!("invalid {key}: value must be positive"));
        }
        Ok(parsed)
    }

    fn optional_positive_u64(&self, key: &'static str, default: u64) -> Result<u64, String> {
        let Some(values) = self.values.get(key) else {
            return Ok(default);
        };
        if values.len() != 1 {
            return Err(format!("config key {key} must appear once"));
        }
        let value = values
            .first()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {key}"))?;
        let parsed = value
            .parse::<u64>()
            .map_err(|error| format!("invalid {key}: {error}"))?;
        if parsed == 0 {
            return Err(format!("invalid {key}: value must be positive"));
        }
        Ok(parsed)
    }

    fn required_u32(&self, key: &'static str) -> Result<u32, String> {
        self.required(key)?
            .parse::<u32>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn required_positive_u32(&self, key: &'static str) -> Result<u32, String> {
        let value = self.required_u32(key)?;
        if value == u32::default() {
            return Err(format!("invalid {key}: value must be positive"));
        }
        Ok(value)
    }

    pub(super) fn required_u64(&self, key: &'static str) -> Result<u64, String> {
        self.required(key)?
            .parse::<u64>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_positive_u64(&self, key: &'static str) -> Result<u64, String> {
        let value = self.required_u64(key)?;
        if value == u64::default() {
            return Err(format!("invalid {key}: value must be positive"));
        }
        Ok(value)
    }

    fn required_disabled_or_positive_u64(&self, key: &'static str) -> Result<Option<u64>, String> {
        let raw = self.required(key)?;
        if raw == "disabled" {
            return Ok(None);
        }
        let value = raw
            .parse::<u64>()
            .map_err(|error| format!("invalid {key}: {error}"))?;
        if value == u64::default() {
            return Err(format!("invalid {key}: value must be positive or disabled"));
        }
        Ok(Some(value))
    }

    pub(super) fn required_octal(&self, key: &'static str) -> Result<u32, String> {
        u32::from_str_radix(&self.required(key)?, 8)
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn required_memlock_rlimit(
        &self,
        key: &'static str,
    ) -> Result<MemlockRlimit, String> {
        self.required(key)?
            .parse::<MemlockRlimit>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_tls_library(&self, key: &'static str) -> Result<PayloadTlsLibrary, String> {
        self.required(key)?
            .parse::<PayloadTlsLibrary>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_tls_source(&self, key: &'static str) -> Result<PayloadTlsSource, String> {
        self.required(key)?
            .parse::<PayloadTlsSource>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_tls_resolver(
        &self,
        key: &'static str,
    ) -> Result<PayloadTlsResolver, String> {
        self.required(key)?
            .parse::<PayloadTlsResolver>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_tls_capture_backend(
        &self,
        key: &'static str,
    ) -> Result<PayloadTlsCaptureBackend, String> {
        self.required(key)?
            .parse::<PayloadTlsCaptureBackend>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn payload_tls_seccomp_syscalls(&self) -> Result<Vec<PayloadTlsSeccompSyscall>, String> {
        let Some(values) = self.values.get("payload_tls_seccomp_syscall") else {
            return Err("missing config key payload_tls_seccomp_syscall".to_string());
        };
        values
            .iter()
            .map(|value| {
                value
                    .parse::<PayloadTlsSeccompSyscall>()
                    .map_err(|error| format!("invalid payload_tls_seccomp_syscall: {error}"))
            })
            .collect()
    }

    fn required_payload_socket_capture_backend(
        &self,
        key: &'static str,
    ) -> Result<PayloadSocketCaptureBackend, String> {
        self.required(key)?
            .parse::<PayloadSocketCaptureBackend>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn payload_socket_seccomp_syscalls(&self) -> Result<Vec<PayloadSocketSeccompSyscall>, String> {
        let Some(values) = self.values.get("payload_socket_seccomp_syscall") else {
            return Err("missing config key payload_socket_seccomp_syscall".to_string());
        };
        values
            .iter()
            .map(|value| {
                value
                    .parse::<PayloadSocketSeccompSyscall>()
                    .map_err(|error| format!("invalid payload_socket_seccomp_syscall: {error}"))
            })
            .collect()
    }

    fn required_payload_tls_library_path(
        &self,
        key: &'static str,
    ) -> Result<PayloadTlsLibraryPath, String> {
        self.required(key)?
            .parse::<PayloadTlsLibraryPath>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_tls_sync_runtime_library_path(
        &self,
        key: &'static str,
    ) -> Result<PayloadTlsSyncRuntimeLibraryPath, String> {
        self.required(key)?
            .parse::<PayloadTlsSyncRuntimeLibraryPath>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_disabled_or_path(
        &self,
        key: &'static str,
    ) -> Result<config_core::daemon::DisabledOrPath, String> {
        self.required(key)?
            .parse::<config_core::daemon::DisabledOrPath>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_redaction_policy(
        &self,
        key: &'static str,
    ) -> Result<PayloadRedactionPolicy, String> {
        self.required(key)?
            .parse::<PayloadRedactionPolicy>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_payload_stdio_storage_mode(
        &self,
        key: &'static str,
    ) -> Result<PayloadStdioStorageMode, String> {
        self.required(key)?
            .parse::<PayloadStdioStorageMode>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_sse_data_policy(&self, key: &'static str) -> Result<SseDataPolicy, String> {
        self.required(key)?
            .parse::<SseDataPolicy>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_enforcement_backend(
        &self,
        key: &'static str,
    ) -> Result<EnforcementBackend, String> {
        self.required(key)?
            .parse::<EnforcementBackend>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_enforcement_scope(&self, key: &'static str) -> Result<EnforcementScope, String> {
        self.required(key)?
            .parse::<EnforcementScope>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_enforcement_decision(
        &self,
        key: &'static str,
    ) -> Result<EnforcementDecision, String> {
        self.required(key)?
            .parse::<EnforcementDecision>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    fn required_enforcement_mark_strategy(
        &self,
        key: &'static str,
    ) -> Result<EnforcementMarkStrategy, String> {
        self.required(key)?
            .parse::<EnforcementMarkStrategy>()
            .map_err(|error| format!("invalid {key}: {error}"))
    }

    pub(super) fn payload_tls_config(&self) -> Result<PayloadTlsConfig, String> {
        Ok(PayloadTlsConfig {
            enabled: self.required_bool("payload_tls_enabled")?,
            capture_backend: self
                .required_payload_tls_capture_backend("payload_tls_capture_backend")?,
            source: self.required_payload_tls_source("payload_tls_source")?,
            resolver: self.required_payload_tls_resolver("payload_tls_resolver")?,
            library: self.required_payload_tls_library("payload_tls_library")?,
            library_path: self.required_payload_tls_library_path("payload_tls_library_path")?,
            binary_path: self.required_disabled_or_path("payload_tls_binary_path")?,
            pattern_path: self.required_disabled_or_path("payload_tls_pattern_path")?,
            max_segment_bytes: self.required_positive_u32("payload_tls_max_segment_bytes")?,
            max_operation_bytes: self.required_positive_u32("payload_tls_max_operation_bytes")?,
            ring_buffer_bytes: self.required_positive_u32("payload_tls_ring_buffer_bytes")?,
            pending_operation_max_entries: self
                .required_positive_u32("payload_tls_pending_operation_max_entries")?,
            seccomp_syscalls: self.payload_tls_seccomp_syscalls()?,
            diagnostics_enabled: self.optional_bool("payload_tls_diagnostics_enabled", false)?,
            retention_max_bytes_per_trace: self
                .required_positive_u64("payload_tls_retention_max_bytes_per_trace")?,
            redaction_policy: self
                .required_payload_redaction_policy("payload_tls_redaction_policy")?,
            sync_runtime_library_path: self.required_payload_tls_sync_runtime_library_path(
                "payload_tls_sync_runtime_library_path",
            )?,
            sync_event_socket_path: self.required_path("payload_tls_sync_event_socket_path")?,
            sync_socket_mode: self.required_octal("payload_tls_sync_socket_mode_octal")?,
            sync_match_limit: self.required_positive_u32("payload_tls_sync_match_limit")?,
            sync_flow_control_enabled: self
                .optional_bool("payload_tls_sync_flow_control_enabled", true)?,
            sync_flow_sniff_bytes: self
                .optional_positive_u32("payload_tls_sync_flow_sniff_bytes", 65536)?,
            sync_flow_max_header_bytes: self
                .optional_positive_u32("payload_tls_sync_flow_max_header_bytes", 16384)?,
            sync_flow_large_transfer_bytes: self
                .optional_positive_u64("payload_tls_sync_flow_large_transfer_bytes", 1048576)?,
            sync_flow_unknown_stream_bytes: self
                .optional_positive_u64("payload_tls_sync_flow_unknown_stream_bytes", 65536)?,
            sync_flow_h2_data_probe_bytes: self
                .optional_positive_u64("payload_tls_sync_flow_h2_data_probe_bytes", 65536)?,
            java_agent_enabled: self.optional_bool("payload_tls_java_agent_enabled", false)?,
        })
    }

    pub(super) fn payload_stdio_config(&self) -> Result<PayloadStdioConfig, String> {
        Ok(PayloadStdioConfig {
            enabled: self.required_bool("payload_stdio_enabled")?,
            capture_stdin: self.required_bool("payload_stdio_capture_stdin")?,
            capture_stdout: self.required_bool("payload_stdio_capture_stdout")?,
            capture_stderr: self.required_bool("payload_stdio_capture_stderr")?,
            stdin_storage_mode: self
                .required_payload_stdio_storage_mode("payload_stdio_stdin_storage_mode")?,
            stdout_storage_mode: self
                .required_payload_stdio_storage_mode("payload_stdio_stdout_storage_mode")?,
            stderr_storage_mode: self
                .required_payload_stdio_storage_mode("payload_stdio_stderr_storage_mode")?,
            max_segment_bytes: self.required_positive_u32("payload_stdio_max_segment_bytes")?,
            ring_buffer_bytes: self.required_positive_u32("payload_stdio_ring_buffer_bytes")?,
            pending_operation_max_entries: self
                .required_positive_u32("payload_stdio_pending_operation_max_entries")?,
            stream_state_max_entries: self
                .required_positive_u32("payload_stdio_stream_state_max_entries")?,
            retention_max_bytes_per_trace: self
                .required_positive_u64("payload_stdio_retention_max_bytes_per_trace")?,
            redaction_policy: self
                .required_payload_redaction_policy("payload_stdio_redaction_policy")?,
        })
    }

    pub(super) fn payload_socket_config(&self) -> Result<PayloadSocketConfig, String> {
        Ok(PayloadSocketConfig {
            enabled: self.required_bool("payload_socket_enabled")?,
            capture_backend: self
                .required_payload_socket_capture_backend("payload_socket_capture_backend")?,
            max_segment_bytes: self.required_positive_u32("payload_socket_max_segment_bytes")?,
            max_operation_bytes: self
                .required_positive_u32("payload_socket_max_operation_bytes")?,
            ring_buffer_bytes: self.required_positive_u32("payload_socket_ring_buffer_bytes")?,
            pending_operation_max_entries: self
                .required_positive_u32("payload_socket_pending_operation_max_entries")?,
            stream_state_max_entries: self
                .required_positive_u32("payload_socket_stream_state_max_entries")?,
            retention_max_bytes_per_trace: self
                .required_positive_u64("payload_socket_retention_max_bytes_per_trace")?,
            redaction_policy: self
                .required_payload_redaction_policy("payload_socket_redaction_policy")?,
            http_sniff_max_bytes: self
                .required_positive_u64("payload_socket_http_sniff_max_bytes")?,
            seccomp_syscalls: self.payload_socket_seccomp_syscalls()?,
        })
    }

    pub(super) fn application_protocol_config(&self) -> Result<ApplicationProtocolConfig, String> {
        Ok(ApplicationProtocolConfig {
            enabled: self.required_bool("application_protocol_enabled")?,
            http1_enabled: self.required_bool("application_protocol_http1_enabled")?,
            http2_enabled: self.required_bool("application_protocol_http2_enabled")?,
            capture_host: self.required_bool("application_http_capture_host")?,
            sse_enabled: self.required_bool("application_http_sse_enabled")?,
            sse_data_policy: self.required_sse_data_policy("application_http_sse_data_policy")?,
            sse_max_buffer_bytes: self
                .required_positive_u64("application_http_sse_max_buffer_bytes")?,
            sse_max_data_bytes: self
                .required_positive_u64("application_http_sse_max_data_bytes")?,
            http2_max_frame_bytes: self
                .required_positive_u64("application_http2_max_frame_bytes")?,
            http2_max_connection_buffer_bytes: self
                .required_positive_u64("application_http2_max_connection_buffer_bytes")?,
            http2_emit_data_preview: self.required_bool("application_http2_emit_data_preview")?,
            http2_max_data_preview_bytes: self
                .required_positive_u64("application_http2_max_data_preview_bytes")?,
        })
    }

    pub(super) fn resource_metrics_config(&self) -> Result<ResourceMetricsConfig, String> {
        Ok(ResourceMetricsConfig {
            enabled: self.required_bool("resource_metrics_enabled")?,
            interval_ms: self.required_positive_u64("resource_metrics_interval_ms")?,
            include_children: self.required_bool("resource_metrics_include_children")?,
            include_system: self.required_bool("resource_metrics_include_system")?,
            cpu_alert_percent_millis: self
                .required_disabled_or_positive_u64("resource_metrics_cpu_alert_percent_millis")?,
            memory_alert_rss_kb: self
                .required_disabled_or_positive_u64("resource_metrics_memory_alert_rss_kb")?,
        })
    }

    pub(super) fn enforcement_config(&self) -> Result<EnforcementConfig, String> {
        Ok(EnforcementConfig {
            enabled: self.required_bool("enforcement_enabled")?,
            backend: self.required_enforcement_backend("enforcement_backend")?,
            scope: self.required_enforcement_scope("enforcement_scope")?,
            rules_path: self.required_path("enforcement_rules_path")?,
            builtin_rules: Vec::new(),
            default_decision: self.required_enforcement_decision("enforcement_default_decision")?,
            mark_strategy: self.required_enforcement_mark_strategy("enforcement_mark_strategy")?,
            audit_enabled: self.required_bool("enforcement_audit_enabled")?,
            event_buffer_bytes: self.required_positive_u32("enforcement_event_buffer_bytes")?,
        })
    }

    pub(super) fn optional_mmap_config(&self) -> Result<Option<MmapWorkloadConfig>, String> {
        if !self.required_bool("mmap_enabled")? {
            self.reject_disabled_mmap_keys()?;
            return Ok(None);
        }
        Ok(Some(MmapWorkloadConfig {
            path: self.required_path("mmap_path")?,
            message: self.required("mmap_message")?,
            length: self.required_positive_u64("mmap_length")?,
            offset: self.required_u64("mmap_offset")?,
        }))
    }

    fn reject_disabled_mmap_keys(&self) -> Result<(), String> {
        for key in ["mmap_path", "mmap_message", "mmap_length", "mmap_offset"] {
            if self.values.contains_key(key) {
                return Err(format!("{key} is only valid when mmap_enabled = true"));
            }
        }
        Ok(())
    }
}

fn unquote(value: &str) -> Result<String, String> {
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
            return Err(format!("invalid quoted value {value}"));
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}

use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ControlDocument {
    pub socket_path: String,
    pub socket_mode_octal: String,
    pub pending_connection_max: u32,
    pub active_trace_max: u32,
    pub pid_file: String,
    pub log_path: String,
    pub diagnostic_log_level: String,
    pub workload_diagnostics: WorkloadDiagnosticsDocument,
    pub finalization: FinalizationDocument,
}

impl Default for ControlDocument {
    fn default() -> Self {
        Self {
            socket_path: "/run/actrail/control.sock".to_string(),
            socket_mode_octal: "660".to_string(),
            pending_connection_max: DEFAULT_CONTROL_PENDING_CONNECTION_MAX,
            active_trace_max: DEFAULT_ACTIVE_TRACE_MAX,
            pid_file: "/run/actrail/actraild.pid".to_string(),
            log_path: "/var/log/actrail/actraild.log".to_string(),
            diagnostic_log_level: "info".to_string(),
            workload_diagnostics: WorkloadDiagnosticsDocument::default(),
            finalization: FinalizationDocument::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct WorkloadDiagnosticsDocument {
    pub enabled: bool,
    pub interval_ms: u64,
}

impl Default for WorkloadDiagnosticsDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: 1000,
        }
    }
}

impl WorkloadDiagnosticsDocument {
    pub(super) fn to_config(&self) -> Result<WorkloadDiagnosticsConfig, String> {
        Ok(WorkloadDiagnosticsConfig {
            enabled: self.enabled,
            interval_ms: require_positive_u64(
                "control.workload_diagnostics.interval_ms",
                self.interval_ms,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FinalizationDocument {
    pub traces_per_cycle: u32,
    pub poll_interval_ms: u64,
}

impl Default for FinalizationDocument {
    fn default() -> Self {
        Self {
            traces_per_cycle: DEFAULT_FINALIZATION_TRACES_PER_CYCLE,
            poll_interval_ms: DEFAULT_FINALIZATION_POLL_INTERVAL_MS,
        }
    }
}

impl FinalizationDocument {
    pub(super) fn to_config(&self) -> Result<TraceFinalizationConfig, String> {
        Ok(TraceFinalizationConfig {
            traces_per_cycle: require_positive_u32(
                "control.finalization.traces_per_cycle",
                self.traces_per_cycle,
            )?,
            poll_interval_ms: require_positive_u64(
                "control.finalization.poll_interval_ms",
                self.poll_interval_ms,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StorageDocument {
    pub backend: String,
    pub sqlite: SqliteStorageDocument,
    pub retention: StorageRetentionDocument,
}

impl Default for StorageDocument {
    fn default() -> Self {
        Self {
            backend: "sqlite".to_string(),
            sqlite: SqliteStorageDocument::default(),
            retention: StorageRetentionDocument::default(),
        }
    }
}

impl StorageDocument {
    pub(super) fn to_config(&self) -> Result<StorageConfig, String> {
        if self.backend != "sqlite" {
            return Err(format!(
                "invalid storage.backend: expected sqlite, got {}",
                self.backend
            ));
        }
        Ok(StorageConfig::sqlite(
            &self.sqlite.path,
            require_positive_u64(
                "storage.sqlite.busy_timeout_ms",
                self.sqlite.busy_timeout_ms,
            )?,
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SqliteStorageDocument {
    pub path: String,
    pub busy_timeout_ms: u64,
}

impl Default for SqliteStorageDocument {
    fn default() -> Self {
        Self {
            path: "/var/lib/actrail/actrail.sqlite".to_string(),
            busy_timeout_ms: 5000,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StorageRetentionDocument {
    pub enabled: bool,
    pub max_trace_age: String,
    pub sweep_interval: String,
    pub min_terminal_age: String,
    pub max_traces_per_sweep: u32,
    pub protected_tags: Vec<String>,
    pub checkpoint_after_sweep: bool,
}

impl Default for StorageRetentionDocument {
    fn default() -> Self {
        let config = StorageRetentionConfig::default();
        Self::from_config(&config)
    }
}

impl StorageRetentionDocument {
    pub(super) fn from_config(config: &StorageRetentionConfig) -> Self {
        Self {
            enabled: config.enabled,
            max_trace_age: duration_as_string(config.max_trace_age),
            sweep_interval: duration_as_string(config.sweep_interval),
            min_terminal_age: duration_as_string(config.min_terminal_age),
            max_traces_per_sweep: config.max_traces_per_sweep,
            protected_tags: config.protected_tags.clone(),
            checkpoint_after_sweep: config.checkpoint_after_sweep,
        }
    }

    pub(super) fn to_config(&self) -> Result<StorageRetentionConfig, String> {
        Ok(StorageRetentionConfig {
            enabled: self.enabled,
            max_trace_age: parse_required_duration(
                "storage.retention.max_trace_age",
                &self.max_trace_age,
            )?,
            sweep_interval: parse_required_duration(
                "storage.retention.sweep_interval",
                &self.sweep_interval,
            )?,
            min_terminal_age: parse_required_duration(
                "storage.retention.min_terminal_age",
                &self.min_terminal_age,
            )?,
            max_traces_per_sweep: require_positive_u32(
                "storage.retention.max_traces_per_sweep",
                self.max_traces_per_sweep,
            )?,
            protected_tags: self.protected_tags.clone(),
            checkpoint_after_sweep: self.checkpoint_after_sweep,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct WebDocument {
    pub listen_addr: String,
    pub request_read_timeout_ms: String,
}

impl Default for WebDocument {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:18080".to_string(),
            request_read_timeout_ms: "1000".to_string(),
        }
    }
}

impl WebDocument {
    pub(super) fn to_config(&self) -> Result<WebServerConfig, String> {
        Ok(WebServerConfig {
            listen_addr: self
                .listen_addr
                .parse()
                .map_err(|error| format!("invalid web.listen_addr: {error}"))?,
            request_read_timeout: parse_duration_millis(
                "web.request_read_timeout_ms",
                &self.request_read_timeout_ms,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ExportDocument {
    pub snapshot: SnapshotExportDocument,
    pub runtime: RuntimeExportDocument,
}

impl Default for ExportDocument {
    fn default() -> Self {
        Self {
            snapshot: SnapshotExportDocument::default(),
            runtime: RuntimeExportDocument::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SnapshotExportDocument {
    pub graph_schema_version: String,
    pub allow_active_trace_snapshot: bool,
    pub directory: String,
    pub payload_bytes_enabled: bool,
    pub payload_text_enabled: bool,
}

impl Default for SnapshotExportDocument {
    fn default() -> Self {
        Self {
            graph_schema_version: "manual-v1".to_string(),
            allow_active_trace_snapshot: true,
            directory: "/var/lib/actrail/export".to_string(),
            payload_bytes_enabled: true,
            payload_text_enabled: true,
        }
    }
}

impl SnapshotExportDocument {
    pub(super) fn to_config(&self) -> ExportConfig {
        ExportConfig {
            graph_schema_version: self.graph_schema_version.clone(),
            allow_active_trace_snapshot: self.allow_active_trace_snapshot,
            output_directory: PathBuf::from(&self.directory),
            payload_bytes_enabled: self.payload_bytes_enabled,
            payload_text_enabled: self.payload_text_enabled,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RuntimeExportDocument {
    pub enabled: bool,
    pub routes: Vec<RuntimeExportRouteDocument>,
}

impl Default for RuntimeExportDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            routes: vec![RuntimeExportRouteDocument::default()],
        }
    }
}

impl RuntimeExportDocument {
    pub(super) fn from_config(config: &RuntimeExportConfig) -> Self {
        Self {
            enabled: config.enabled,
            routes: config
                .routes()
                .iter()
                .map(RuntimeExportRouteDocument::from_config)
                .collect(),
        }
    }

    pub(super) fn to_config(&self) -> Result<RuntimeExportConfig, String> {
        let mut routes = Vec::new();
        for route in &self.routes {
            routes.push(route.to_config()?);
        }
        let config = RuntimeExportConfig::new(self.enabled, routes);
        if self.enabled && config.routes().iter().all(|route| !route.enabled) {
            return Err(
                "export.runtime.enabled=true requires at least one enabled route".to_string(),
            );
        }
        if self.enabled {
            for route in config.routes().iter().filter(|route| route.enabled) {
                route.target.validate_enabled_route()?;
            }
        }
        Ok(config)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RuntimeExportRouteDocument {
    pub name: String,
    pub kind: String,
    pub delivery: String,
    pub enabled: bool,
    pub otel_jsonl: OtelJsonlRouteDocument,
}

impl Default for RuntimeExportRouteDocument {
    fn default() -> Self {
        Self {
            name: "live-otel".to_string(),
            kind: "otel-jsonl".to_string(),
            delivery: "best-effort".to_string(),
            enabled: true,
            otel_jsonl: OtelJsonlRouteDocument::default(),
        }
    }
}

impl RuntimeExportRouteDocument {
    pub(super) fn from_config(config: &ExportRouteConfig) -> Self {
        let ExportRouteTargetConfig::OtelJsonl(otel_jsonl) = &config.target;
        Self {
            name: config.name.clone(),
            kind: config.target.kind().as_str().to_string(),
            delivery: config.delivery.as_str().to_string(),
            enabled: config.enabled,
            otel_jsonl: OtelJsonlRouteDocument {
                path: otel_jsonl.path.display().to_string(),
                overwrite_enabled: otel_jsonl.overwrite_enabled,
                queue_capacity: otel_jsonl.queue_capacity,
                flush_every_spans: otel_jsonl.flush_every_spans,
            },
        }
    }

    pub(super) fn to_config(&self) -> Result<ExportRouteConfig, String> {
        let kind = parse_value::<ExportRouteKind>("export.runtime.routes.kind", &self.kind)?;
        if kind != ExportRouteKind::OtelJsonl {
            return Err(format!("unsupported export route kind {}", self.kind));
        }
        Ok(ExportRouteConfig {
            name: required_non_empty("export.runtime.routes.name", &self.name)?.to_string(),
            enabled: self.enabled,
            delivery: parse_value::<ExportDeliveryConfig>(
                "export.runtime.routes.delivery",
                &self.delivery,
            )?,
            target: ExportRouteTargetConfig::OtelJsonl(OtelJsonlExporterConfig {
                path: PathBuf::from(&self.otel_jsonl.path),
                overwrite_enabled: self.otel_jsonl.overwrite_enabled,
                queue_capacity: require_positive_u32(
                    "export.runtime.routes.otel_jsonl.queue_capacity",
                    self.otel_jsonl.queue_capacity,
                )?,
                flush_every_spans: require_positive_u32(
                    "export.runtime.routes.otel_jsonl.flush_every_spans",
                    self.otel_jsonl.flush_every_spans,
                )?,
            }),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct OtelJsonlRouteDocument {
    pub path: String,
    pub overwrite_enabled: bool,
    pub queue_capacity: u32,
    pub flush_every_spans: u32,
}

impl Default for OtelJsonlRouteDocument {
    fn default() -> Self {
        Self {
            path: "/var/lib/actrail/export/live-spans.otlp.jsonl".to_string(),
            overwrite_enabled: true,
            queue_capacity: 1024,
            flush_every_spans: 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PluginsDocument {
    pub startup: StartupPluginsDocument,
}

impl Default for PluginsDocument {
    fn default() -> Self {
        Self {
            startup: StartupPluginsDocument::default(),
        }
    }
}

impl PluginsDocument {
    pub(super) fn from_config(config: &StartupPluginsConfig) -> Self {
        Self {
            startup: StartupPluginsDocument::from_config(config),
        }
    }

    pub(super) fn to_config(&self) -> Result<StartupPluginsConfig, String> {
        self.startup.to_config()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StartupPluginsDocument {
    pub enabled: bool,
    pub failure_policy: String,
    pub load: Vec<StartupPluginLoadDocument>,
}

impl Default for StartupPluginsDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_policy: StartupPluginFailurePolicy::FailFast.as_str().to_string(),
            load: Vec::new(),
        }
    }
}

impl StartupPluginsDocument {
    pub(super) fn from_config(config: &StartupPluginsConfig) -> Self {
        Self {
            enabled: config.enabled,
            failure_policy: config.failure_policy.as_str().to_string(),
            load: config
                .load
                .iter()
                .map(StartupPluginLoadDocument::from_config)
                .collect(),
        }
    }

    pub(super) fn to_config(&self) -> Result<StartupPluginsConfig, String> {
        let failure_policy = parse_value::<StartupPluginFailurePolicy>(
            "plugins.startup.failure_policy",
            &self.failure_policy,
        )?;
        let mut load = Vec::new();
        for item in &self.load {
            load.push(item.to_config()?);
        }
        if self.enabled && load.iter().all(|item| !item.enabled) {
            return Err(
                "plugins.startup.enabled=true requires at least one enabled load entry".to_string(),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for item in load.iter().filter(|item| item.enabled) {
            if !seen.insert(item.instance_id.clone()) {
                return Err(format!(
                    "plugins.startup.load instance {} is duplicated",
                    item.instance_id
                ));
            }
        }
        Ok(StartupPluginsConfig {
            enabled: self.enabled,
            failure_policy,
            load,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StartupPluginLoadDocument {
    pub instance: String,
    pub enabled: bool,
    pub failure_policy: String,
    pub manifest: String,
    pub plugin_config: String,
    pub host_grants: Vec<String>,
}

impl Default for StartupPluginLoadDocument {
    fn default() -> Self {
        Self {
            instance: String::new(),
            enabled: true,
            failure_policy: String::new(),
            manifest: String::new(),
            plugin_config: String::new(),
            host_grants: Vec::new(),
        }
    }
}

impl StartupPluginLoadDocument {
    fn from_config(config: &StartupPluginLoadConfig) -> Self {
        Self {
            instance: config.instance_id.clone(),
            enabled: config.enabled,
            failure_policy: config
                .failure_policy
                .map(StartupPluginFailurePolicy::as_str)
                .unwrap_or("")
                .to_string(),
            manifest: config.manifest_path.display().to_string(),
            plugin_config: config
                .plugin_config_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            host_grants: config.host_grants.clone(),
        }
    }

    fn to_config(&self) -> Result<StartupPluginLoadConfig, String> {
        let failure_policy = if self.failure_policy.trim().is_empty() {
            None
        } else {
            Some(parse_value::<StartupPluginFailurePolicy>(
                "plugins.startup.load.failure_policy",
                &self.failure_policy,
            )?)
        };
        Ok(StartupPluginLoadConfig {
            instance_id: required_non_empty("plugins.startup.load.instance", &self.instance)?
                .to_string(),
            enabled: self.enabled,
            failure_policy,
            manifest_path: PathBuf::from(required_non_empty(
                "plugins.startup.load.manifest",
                &self.manifest,
            )?),
            plugin_config_path: if self.plugin_config.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.plugin_config))
            },
            host_grants: self.host_grants.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct CaptureDocument {
    pub profile_name: String,
    pub capabilities: Vec<String>,
    pub opportunistic_capabilities: Vec<String>,
    pub disabled_capabilities: Vec<String>,
}

impl Default for CaptureDocument {
    fn default() -> Self {
        Self {
            profile_name: "default-full-monitor".to_string(),
            capabilities: [
                "proc-lifecycle",
                "proc-exec-context",
                "fs-access-basic",
                "fs-mmap",
                "net-transport",
                "ipc-unix-socket",
                "ipc-pipe-fifo",
                "stdio-chunk",
                "tls-plaintext-payload",
                "socket-plaintext-payload",
                "net-application-plaintext-http",
                "net-application-http2-frames",
                "resource-metrics",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            opportunistic_capabilities: Vec::new(),
            disabled_capabilities: Vec::new(),
        }
    }
}

impl CaptureDocument {
    pub(super) fn capability_requests(&self) -> Result<Vec<CapabilityRequest>, String> {
        let mut requests = Vec::new();
        for raw in &self.capabilities {
            requests.push(CapabilityRequest::new(
                parse_capability(raw)?,
                RequestMode::Required,
            ));
        }
        for raw in &self.opportunistic_capabilities {
            requests.push(CapabilityRequest::new(
                parse_capability(raw)?,
                RequestMode::Opportunistic,
            ));
        }
        for raw in &self.disabled_capabilities {
            requests.push(CapabilityRequest::new(
                parse_capability(raw)?,
                RequestMode::Disabled,
            ));
        }
        Ok(requests)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct EbpfDocument {
    /// "true" | "false" | "auto". Accepted as either a quoted string
    /// (`enabled = "auto"`) or a bare boolean (`enabled = true`) for
    /// backward compatibility with configs written before `auto` existed;
    /// normalized to a string and parsed into `EbpfEnabledMode` in `to_config`.
    #[serde(
        deserialize_with = "deserialize_ebpf_enabled",
        serialize_with = "serialize_ebpf_enabled"
    )]
    pub enabled: String,
    pub memlock_rlimit: String,
    pub tracked_process_max_entries: u32,
    pub pending_operation_max_entries: u32,
    pub suppressed_fd_max_entries: u32,
    pub suppressed_fd_index_slots_per_process: u32,
    pub event_ring_buffer_max_bytes: u32,
    pub file_path_capture_enabled: bool,
    pub file_path_max_bytes: u32,
}

impl Default for EbpfDocument {
    fn default() -> Self {
        Self {
            enabled: "true".to_string(),
            memlock_rlimit: "inherit".to_string(),
            tracked_process_max_entries: 8192,
            pending_operation_max_entries: 8192,
            suppressed_fd_max_entries: 8192,
            suppressed_fd_index_slots_per_process: 64,
            event_ring_buffer_max_bytes: 33554432,
            file_path_capture_enabled: true,
            file_path_max_bytes: 255,
        }
    }
}

impl EbpfDocument {
    pub(super) fn to_config(&self) -> Result<EbpfCollectorConfig, String> {
        let enabled_mode = self
            .enabled
            .parse::<EbpfEnabledMode>()
            .map_err(|error| format!("ebpf.enabled: {error}"))?;
        // At parse time `enabled` is true only for an explicit `true`; `auto`
        // defers to daemon-side resolution (starts false).
        let enabled = matches!(enabled_mode, EbpfEnabledMode::True);
        Ok(EbpfCollectorConfig {
            enabled_mode,
            enabled,
            memlock_rlimit: parse_value::<MemlockRlimit>(
                "ebpf.memlock_rlimit",
                &self.memlock_rlimit,
            )?,
            tracked_process_max_entries: self.tracked_process_max_entries,
            pending_operation_max_entries: self.pending_operation_max_entries,
            suppressed_fd_max_entries: self.suppressed_fd_max_entries,
            suppressed_fd_index_slots_per_process: self.suppressed_fd_index_slots_per_process,
            event_ring_buffer_max_bytes: self.event_ring_buffer_max_bytes,
            file_path_capture_enabled: self.file_path_capture_enabled,
            file_path_max_bytes: require_positive_u32(
                "ebpf.file_path_max_bytes",
                self.file_path_max_bytes,
            )?,
        })
    }
}

/// Deserialize `ebpf.enabled` accepting either a bare boolean (`true`/`false`,
/// the pre-`auto` form still present in shipped example configs) or a quoted
/// string (`"true"`/`"false"`/`"auto"`). Both normalize to a string.
fn deserialize_ebpf_enabled<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum EnabledValue {
        Bool(bool),
        String(String),
    }
    match EnabledValue::deserialize(deserializer)? {
        EnabledValue::Bool(value) => Ok(value.to_string()),
        EnabledValue::String(value) => Ok(value),
    }
}

/// Serialize `ebpf.enabled` back as a quoted string so round-trips produce
/// `enabled = "true"` regardless of how it was written.
fn serialize_ebpf_enabled<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value)
}

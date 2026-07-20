//! Daemon-wide configuration for storage, control plane, collectors, and diagnostics.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use storage_factory::StorageConfig;

#[path = "daemon/agent.rs"]
mod agent;
#[path = "daemon/application.rs"]
mod application;
#[path = "daemon/cluster.rs"]
mod cluster;
#[path = "daemon/command.rs"]
mod command;
#[path = "daemon/enforcement.rs"]
mod enforcement;
#[path = "daemon/observation/file.rs"]
mod file_observation;
#[path = "daemon/logging/config.rs"]
mod logging;
#[path = "daemon/network.rs"]
mod network;
#[path = "daemon/operator.rs"]
mod operator;
#[path = "daemon/payload.rs"]
mod payload;
#[path = "daemon/process.rs"]
mod process;
#[path = "daemon/resource.rs"]
mod resource;

pub use crate::retention::{
    DEFAULT_STORAGE_RETENTION_ENABLED, DEFAULT_STORAGE_RETENTION_MAX_TRACE_AGE,
    DEFAULT_STORAGE_RETENTION_MIN_TERMINAL_AGE, DEFAULT_STORAGE_RETENTION_SWEEP_INTERVAL,
    DEFAULT_STORAGE_RETENTION_TRACES_PER_SWEEP, StorageRetentionConfig,
};
pub use agent::{
    AgentInvocationConfig, Http2DataContentRetention, HttpBodyRetention, HttpHeadersRetention,
    L0LlmCallRetention, L1SseRetention, L2HttpRetention, L3Http2FrameRetention, L4PayloadRetention,
    LlmRequestContentRetention, LlmResponseContentRetention, LlmToolCallRetention,
    LlmUsageRetention, PayloadBodyContentRetention, SemanticContentOwner, SemanticRetentionConfig,
    SseEventContentRetention,
};
pub use application::{ApplicationProtocolConfig, SseDataPolicy};
pub use cluster::{
    ClusterCenterConfig, ClusterConfig, ClusterReportConfig,
    DEFAULT_CLUSTER_REPORT_BATCH_MAX_TRACES, DEFAULT_CLUSTER_REPORT_BUNDLE_RETENTION_DAYS,
    DEFAULT_CLUSTER_REPORT_INTERVAL_SECS, DEFAULT_CLUSTER_REPORT_MAX_RETRY_BACKOFF_SECS,
    DEFAULT_CLUSTER_REPORT_RETRY_BACKOFF_SECS, DEFAULT_CLUSTER_REPORT_UPLOAD_TIMEOUT_SECS,
};
pub use command::CommandControlConfig;
pub use enforcement::{
    EnforcementBackend, EnforcementBuiltinRuleConfig, EnforcementConfig, EnforcementDecision,
    EnforcementMarkStrategy, EnforcementScope,
};
pub use export_factory::ExportConfig as RuntimeExportConfig;
pub use file_observation::{
    DEFAULT_FILE_BULK_READ_MAX_PATHS_PER_SET, DEFAULT_FILE_BULK_READ_MIN_UNIQUE_PATHS,
    DEFAULT_FILE_BULK_READ_PENDING_EVENT_MAX, DEFAULT_FS_ENUMERATE_MAX_PATHS_PER_SET,
    DEFAULT_FS_ENUMERATE_MIN_UNIQUE_PATHS, FileBulkReadFastPathConfig, FileBulkReadMode,
    FileBulkReadObservationConfig, FileMetadataRetention, FileObservationConfig,
    FileRawEventRetention, FileTtyObservationConfig, FsEnumerateObservationConfig,
};
pub use logging::{
    DEFAULT_WORKLOAD_DIAGNOSTICS_ENABLED, DEFAULT_WORKLOAD_DIAGNOSTICS_INTERVAL_MS,
    DiagnosticLogLevel, WorkloadDiagnosticsConfig,
};
pub use network::{NetworkControlConfig, NetworkControlSeccompSyscall};
pub use operator::{
    DEFAULT_ACTIVE_TRACE_MAX, DEFAULT_CONTROL_PENDING_CONNECTION_MAX, DEFAULT_OPERATOR_CONFIG_PATH,
    OperatorConfig, OperatorConfigInitStatus, StartupPluginFailurePolicy, StartupPluginLoadConfig,
    StartupPluginsConfig, launch_seccomp_requirements,
};
pub use payload::{
    DisabledOrPath, PayloadConfig, PayloadRedactionPolicy, PayloadSocketCaptureBackend,
    PayloadSocketConfig, PayloadSocketSeccompSyscall, PayloadStdioConfig, PayloadStdioStorageMode,
    PayloadTlsCaptureBackend, PayloadTlsConfig, PayloadTlsLibrary, PayloadTlsLibraryPath,
    PayloadTlsResolver, PayloadTlsSeccompSyscall, PayloadTlsSource,
    PayloadTlsSyncRuntimeLibraryPath,
};
pub use process::{ProcessSeccompConfig, ProcessSeccompSyscall, SeccompNotifyConfig};
pub use resource::ResourceMetricsConfig;

pub const DEFAULT_FINALIZATION_TRACES_PER_CYCLE: u32 = 1;
pub const DEFAULT_FINALIZATION_POLL_INTERVAL_MS: u64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceFinalizationConfig {
    pub traces_per_cycle: u32,
    pub poll_interval_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebServerConfig {
    pub listen_addr: SocketAddr,
    pub request_read_timeout: Option<Duration>,
}

impl Default for TraceFinalizationConfig {
    fn default() -> Self {
        Self {
            traces_per_cycle: DEFAULT_FINALIZATION_TRACES_PER_CYCLE,
            poll_interval_ms: DEFAULT_FINALIZATION_POLL_INTERVAL_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketPermissions {
    pub mode: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemlockRlimit {
    Inherit,
    Unlimited,
    Bytes(u64),
}

impl FromStr for MemlockRlimit {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "inherit" {
            return Ok(Self::Inherit);
        }
        if value == "unlimited" {
            return Ok(Self::Unlimited);
        }
        let Some(bytes) = value.strip_prefix("bytes:") else {
            return Err("expected inherit, unlimited, or bytes:<n>".to_string());
        };
        bytes
            .parse::<u64>()
            .map(Self::Bytes)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EbpfEnabledMode {
    True,
    False,
    Auto,
}

impl EbpfEnabledMode {
    /// Whether the daemon should probe eBPF at startup and auto-degrade when
    /// the host cannot run eBPF. Only `Auto` defers the decision.
    pub fn defers_to_runtime(self) -> bool {
        matches!(self, Self::Auto)
    }
}

impl std::str::FromStr for EbpfEnabledMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "true" => Ok(Self::True),
            "false" => Ok(Self::False),
            "auto" => Ok(Self::Auto),
            _ => Err(format!(
                "invalid ebpf.enabled: expected true, false, or auto, got {value}"
            )),
        }
    }
}

impl std::fmt::Display for EbpfEnabledMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::True => "true",
            Self::False => "false",
            Self::Auto => "auto",
        };
        formatter.write_str(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EbpfCollectorConfig {
    /// The operator-declared mode (true/false/auto). Parse-time only; the
    /// daemon resolves `auto` against the host at startup.
    pub enabled_mode: EbpfEnabledMode,
    /// Effective enabled flag after daemon resolution. At parse time this is
    /// `true` only when `enabled_mode == True`; `Auto` starts `false` and is
    /// set by `resolve_ebpf_collector_config` based on the host probe.
    pub enabled: bool,
    pub memlock_rlimit: MemlockRlimit,
    pub tracked_process_max_entries: u32,
    pub pending_operation_max_entries: u32,
    pub suppressed_fd_max_entries: u32,
    pub suppressed_fd_index_slots_per_process: u32,
    pub event_ring_buffer_max_bytes: u32,
    pub file_path_capture_enabled: bool,
    pub file_path_max_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectorConfig {
    pub ebpf: EbpfCollectorConfig,
    pub payload_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonConfig {
    pub storage: StorageConfig,
    pub control_socket_path: PathBuf,
    pub control_socket_permissions: SocketPermissions,
    pub plugin_directories: Vec<PathBuf>,
    pub collector: CollectorConfig,
    pub retention_policy_name: String,
    pub export_policy_name: String,
}

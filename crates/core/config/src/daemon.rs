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
#[path = "daemon/finalization/config.rs"]
mod trace_finalization;

pub use crate::retention::{
    DEFAULT_STORAGE_RETENTION_ENABLED, DEFAULT_STORAGE_RETENTION_MAX_TRACE_AGE,
    DEFAULT_STORAGE_RETENTION_MIN_TERMINAL_AGE, DEFAULT_STORAGE_RETENTION_SWEEP_INTERVAL,
    DEFAULT_STORAGE_RETENTION_TRACES_PER_SWEEP, StorageRetentionConfig,
};
pub use agent::{
    AgentInvocationConfig, DEFAULT_HTTP_EXCHANGE_MAX_PENDING_REQUESTS_PER_STREAM,
    DEFAULT_HTTP_EXCHANGE_MAX_PENDING_RESPONSES_PER_STREAM,
    DEFAULT_HTTP_EXCHANGE_RESPONSE_LATENESS, DEFAULT_LLM_ASSEMBLY_MAX_BUFFER_BYTES,
    DEFAULT_LLM_ASSEMBLY_MAX_SEGMENT_RANGES, DEFAULT_LLM_PROJECTION_MAX_ACTION_VERSIONS_PER_TRACE,
    DEFAULT_LLM_PROJECTION_MAX_ACTIVE_RESPONSE_BINDINGS_PER_TRACE,
    DEFAULT_LLM_PROJECTION_MAX_CORRELATION_STREAMS_PER_TRACE,
    DEFAULT_LLM_PROJECTION_MAX_DAMAGED_RESPONSE_BINDINGS_PER_TRACE,
    DEFAULT_LLM_PROJECTION_MAX_PENDING_REQUESTS_PER_STREAM,
    DEFAULT_LLM_PROJECTION_MAX_PENDING_RESPONSES_PER_STREAM,
    DEFAULT_LLM_PROJECTION_MAX_PENDING_TRAJECTORY_ACTIONS_PER_TRACE,
    DEFAULT_LLM_PROJECTION_MAX_TOOL_ENTRIES_PER_TRACE, DEFAULT_LLM_REQUEST_BODY_EXPORT_MAX_BYTES,
    DEFAULT_LLM_STREAM_CLASSIFIER_SOFT_SNIFF_MAX_BYTES, DEFAULT_LLM_TOOL_RESULT_EXPORT_MAX_BYTES,
    DEFAULT_LLM_TRAJECTORY_IDLE_TTL, DEFAULT_LLM_TRAJECTORY_MAX_ACTIVE_TRAJECTORIES_PER_SCOPE,
    DEFAULT_LLM_TRAJECTORY_MAX_BLOCKS_PER_ATOM,
    DEFAULT_LLM_TRAJECTORY_MAX_CANDIDATE_NODES_PER_TRAJECTORY,
    DEFAULT_LLM_TRAJECTORY_MAX_HISTORY_ATOMS_PER_REQUEST,
    DEFAULT_LLM_TRAJECTORY_MAX_PREFIX_NODES_PER_SCOPE,
    DEFAULT_LLM_TRAJECTORY_MAX_STRUCTURAL_BYTES_PER_ATOM, Http2DataContentRetention,
    HttpBodyRetention, HttpExchangeConfig, HttpHeadersRetention, L0LlmCallRetention,
    L0McpCallRetention, L1SseRetention, L2HttpRetention, L3Http2FrameRetention, L4PayloadRetention,
    LlmAssemblyConfig, LlmProjectionStateConfig, LlmRequestBodyExportRetention,
    LlmRequestContentRetention, LlmResponseContentRetention, LlmStreamClassifierConfig,
    LlmToolCallRetention, LlmToolResultContentExportRetention, LlmTrajectoryConfig,
    LlmUsageRetention, McpJsonRpcContentRetention, PayloadBodyContentRetention,
    SemanticContentOwner, SemanticRetentionConfig, SseEventContentRetention,
};
pub use application::{ApplicationProtocolConfig, SseDataPolicy};
pub use cluster::{
    ClusterCenterConfig, ClusterConfig, ClusterReportConfig,
    DEFAULT_CLUSTER_REPORT_BATCH_MAX_TRACES, DEFAULT_CLUSTER_REPORT_BUNDLE_RETENTION_DAYS,
    DEFAULT_CLUSTER_REPORT_INTERVAL_SECS, DEFAULT_CLUSTER_REPORT_MAX_RETRY_BACKOFF_SECS,
    DEFAULT_CLUSTER_REPORT_RETRY_BACKOFF_SECS, DEFAULT_CLUSTER_REPORT_UPLOAD_TIMEOUT_SECS,
};
pub use command::{
    CommandControlConfig, CommandControlGrayConfig, DEFAULT_COMMAND_ARGV_MAX_ARG_BYTES,
    DEFAULT_COMMAND_ARGV_MAX_COUNT, DEFAULT_COMMAND_ARGV_MAX_TOTAL_BYTES,
    DEFAULT_COMMAND_GRAY_CONCURRENCY_LIMIT, DEFAULT_COMMAND_GRAY_TIMEOUT_MS,
    DEFAULT_COMMAND_PATH_MAX_BYTES, DEFAULT_COMMAND_PENDING_DECISION_MAX,
    DEFAULT_COMMAND_REUSABLE_CACHE_MAX_ENTRIES,
};
pub use enforcement::{
    EnforcementBackend, EnforcementBuiltinRuleConfig, EnforcementConfig, EnforcementDecision,
    EnforcementMarkStrategy, EnforcementScope, EnforcementSeccompSyscall,
};
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
    DEFAULT_PLUGIN_ALERT_DRAIN_TIMEOUT_MS, DEFAULT_PLUGIN_ALERT_QUEUE_CAPACITY,
    DEFAULT_PLUGIN_ALERT_WRITES_PER_CYCLE, DEFAULT_PLUGIN_DISCOVERY_DIRECTORY,
    DEFAULT_PLUGIN_DISCOVERY_MANIFEST_MAX_BYTES, DEFAULT_PLUGIN_DISCOVERY_MAX_PACKAGES,
    HandObservationConfig, OperatorConfig, OperatorConfigInitStatus, PluginAlertRuntimeConfig,
    PluginDiscoveryConfig, SandboxEvidenceConfig, SandboxEvidenceSynchronousConfig,
    StartupPluginFailurePolicy, StartupPluginLoadConfig, StartupPluginsConfig,
    launch_seccomp_requirements,
};
pub use payload::{
    DEFAULT_MCP_PARSE_BUFFER_MAX_BYTES, DEFAULT_MCP_PENDING_STDIO_CANDIDATE_MAX_ENTRIES,
    DEFAULT_MCP_STDIO_CANDIDATE_MAX_BYTES, DEFAULT_TLS_DYNAMIC_EXEC_PLAN_TIMEOUT_MS,
    DisabledOrPath, PayloadConfig, PayloadMcpConfig, PayloadRedactionPolicy,
    PayloadSocketCaptureBackend, PayloadSocketConfig, PayloadSocketSeccompSyscall,
    PayloadStdioConfig, PayloadStdioStorageMode, PayloadTlsCaptureBackend, PayloadTlsConfig,
    PayloadTlsLibrary, PayloadTlsLibraryPath, PayloadTlsResolver, PayloadTlsSeccompSyscall,
    PayloadTlsSource, PayloadTlsSyncRuntimeLibraryPath,
};
pub use process::{ProcessSeccompConfig, ProcessSeccompSyscall, SeccompNotifyConfig};
pub use resource::ResourceMetricsConfig;
pub use trace_finalization::{
    DEFAULT_POST_TRACE_ADMISSION_TIMEOUT_MS, DEFAULT_POST_TRACE_BROKER_QUEUE_CAPACITY,
    DEFAULT_POST_TRACE_BROKER_REPLY_TIMEOUT_MS, DEFAULT_POST_TRACE_EXECUTION_TIMEOUT_MS,
    DEFAULT_POST_TRACE_MAX_IN_FLIGHT_TASKS, DEFAULT_POST_TRACE_REQUESTS_PER_CYCLE,
    DEFAULT_POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS, PostTraceRuntimeConfig,
};

pub const DEFAULT_FINALIZATION_TRACES_PER_CYCLE: u32 = 1;
pub const DEFAULT_FINALIZATION_POLL_INTERVAL_MS: u64 = 100;
pub const DEFAULT_FINALIZATION_SETTLE_DELAY_MS: u64 = 250;
pub const DEFAULT_FINALIZATION_SHUTDOWN_DRAIN_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_EBPF_PREFLIGHT_LINK_TEARDOWN_WORKERS: u32 = 4;
pub const MAX_EBPF_PREFLIGHT_LINK_TEARDOWN_WORKERS: u32 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceFinalizationConfig {
    pub traces_per_cycle: u32,
    pub poll_interval_ms: u64,
    pub settle_delay_ms: u64,
    pub shutdown_drain_timeout_ms: u64,
    pub post_trace: PostTraceRuntimeConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebServerConfig {
    pub listen_addr: SocketAddr,
    pub request_read_timeout: Option<Duration>,
    pub alerts: WebAlertsConfig,
}

pub const DEFAULT_WEB_ALERTS_LIMIT: u32 = 30;
pub const DEFAULT_WEB_ALERTS_MAX_LIMIT: u32 = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebAlertsConfig {
    pub default_limit: u32,
    pub max_limit: u32,
}

impl Default for WebAlertsConfig {
    fn default() -> Self {
        Self {
            default_limit: DEFAULT_WEB_ALERTS_LIMIT,
            max_limit: DEFAULT_WEB_ALERTS_MAX_LIMIT,
        }
    }
}

impl Default for TraceFinalizationConfig {
    fn default() -> Self {
        Self {
            traces_per_cycle: DEFAULT_FINALIZATION_TRACES_PER_CYCLE,
            poll_interval_ms: DEFAULT_FINALIZATION_POLL_INTERVAL_MS,
            settle_delay_ms: DEFAULT_FINALIZATION_SETTLE_DELAY_MS,
            shutdown_drain_timeout_ms: DEFAULT_FINALIZATION_SHUTDOWN_DRAIN_TIMEOUT_MS,
            post_trace: PostTraceRuntimeConfig::default(),
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

/// Maximum process lineage records retained for one trace by default.
pub const DEFAULT_IPC_LINEAGE_MAX_PROCESSES_PER_TRACE: u32 = 8_192;
/// Maximum candidate IPC file descriptors retained for one trace by default.
pub const DEFAULT_IPC_LINEAGE_MAX_CANDIDATE_FDS_PER_TRACE: u32 = 65_536;
/// Maximum complete stdio bundles retained for one trace by default.
pub const DEFAULT_IPC_LINEAGE_MAX_STDIO_BUNDLES_PER_TRACE: u32 = 8_192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcLineageConfig {
    /// Whether IPC pipe/FIFO and Unix-socket observation is enabled.
    pub enabled: bool,
    /// Maximum process lineage records retained for one trace.
    pub max_processes_per_trace: u32,
    /// Maximum candidate IPC file descriptors retained for one trace.
    pub max_candidate_fds_per_trace: u32,
    /// Maximum complete stdio bundles retained for one trace.
    pub max_stdio_bundles_per_trace: u32,
}

impl Default for IpcLineageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_processes_per_trace: DEFAULT_IPC_LINEAGE_MAX_PROCESSES_PER_TRACE,
            max_candidate_fds_per_trace: DEFAULT_IPC_LINEAGE_MAX_CANDIDATE_FDS_PER_TRACE,
            max_stdio_bundles_per_trace: DEFAULT_IPC_LINEAGE_MAX_STDIO_BUNDLES_PER_TRACE,
        }
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
    pub preflight_link_teardown_workers: u32,
    pub tracked_process_max_entries: u32,
    pub pending_operation_max_entries: u32,
    /// Maximum simultaneously tracked descriptors for one process. This is
    /// a dense index capacity and does not limit the numeric fd value.
    pub fd_per_process_max_entries: u32,
    pub suppressed_fd_max_entries: u32,
    pub suppressed_fd_index_slots_per_process: u32,
    pub event_ring_buffer_max_bytes: u32,
    pub file_path_capture_enabled: bool,
    pub file_path_max_bytes: u32,
    /// Aggregate per-connection net send/recv events at the collector (default
    /// on). When enabled, byte totals and event counts are preserved in the
    /// aggregated record; only the number of emitted events is reduced.
    pub net_send_recv_aggregation: bool,
    pub ipc_lineage: IpcLineageConfig,
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

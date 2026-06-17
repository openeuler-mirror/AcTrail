//! Daemon-wide configuration for storage, control plane, collectors, and diagnostics.

use std::path::PathBuf;
use std::str::FromStr;

use storage_factory::StorageConfig;

#[path = "daemon/agent.rs"]
mod agent;
#[path = "daemon/application.rs"]
mod application;
#[path = "daemon/enforcement.rs"]
mod enforcement;
#[path = "daemon/observation/file.rs"]
mod file_observation;
#[path = "daemon/logging/config.rs"]
mod logging;
#[path = "daemon/operator.rs"]
mod operator;
#[path = "daemon/payload.rs"]
mod payload;
#[path = "daemon/process.rs"]
mod process;
#[path = "daemon/resource.rs"]
mod resource;
#[path = "daemon/values.rs"]
mod values;

pub use agent::{
    AgentInvocationConfig, Http2DataContentRetention, HttpBodyRetention, HttpHeadersRetention,
    L0LlmCallRetention, L1SseRetention, L2HttpRetention, L3Http2FrameRetention, L4PayloadRetention,
    LlmRequestContentRetention, LlmResponseContentRetention, LlmToolCallRetention,
    LlmUsageRetention, PayloadBodyContentRetention, SemanticContentOwner, SemanticRetentionConfig,
    SseEventContentRetention,
};
pub use application::{ApplicationProtocolConfig, SseDataPolicy};
pub use enforcement::{
    EnforcementBackend, EnforcementConfig, EnforcementDecision, EnforcementMarkStrategy,
    EnforcementScope,
};
pub use export_factory::ExportConfig as RuntimeExportConfig;
pub use file_observation::{
    DEFAULT_FILE_BULK_READ_MAX_PATHS_PER_SET, DEFAULT_FILE_BULK_READ_MIN_UNIQUE_PATHS,
    DEFAULT_FS_ENUMERATE_MAX_PATHS_PER_SET, DEFAULT_FS_ENUMERATE_MIN_UNIQUE_PATHS,
    FileBulkReadMode, FileBulkReadObservationConfig, FileMetadataRetention, FileObservationConfig,
    FileRawEventRetention, FileTtyObservationConfig, FsEnumerateObservationConfig,
};
pub use logging::DiagnosticLogLevel;
pub use operator::{
    DEFAULT_CONTROL_PENDING_CONNECTION_MAX, DEFAULT_OPERATOR_CONFIG_PATH, OPERATOR_CONFIG_TEMPLATE,
    OperatorConfig, OperatorConfigInitStatus,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EbpfCollectorConfig {
    pub enabled: bool,
    pub memlock_rlimit: MemlockRlimit,
    pub tracked_process_max_entries: u32,
    pub pending_operation_max_entries: u32,
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

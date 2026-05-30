//! Daemon-wide configuration for storage, control plane, collectors, and diagnostics.

use std::path::PathBuf;
use std::str::FromStr;

#[path = "daemon/agent.rs"]
mod agent;
#[path = "daemon/application.rs"]
mod application;
#[path = "daemon/enforcement.rs"]
mod enforcement;
#[path = "daemon/live_otel_export.rs"]
mod live_otel_export;
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

pub use agent::AgentInvocationConfig;
pub use application::{ApplicationProtocolConfig, SseDataPolicy};
pub use enforcement::{
    EnforcementBackend, EnforcementConfig, EnforcementDecision, EnforcementMarkStrategy,
    EnforcementScope,
};
pub use live_otel_export::LiveOtelExportConfig;
pub use logging::DiagnosticLogLevel;
pub use operator::{DEFAULT_OPERATOR_CONFIG_PATH, OPERATOR_CONFIG_TEMPLATE, OperatorConfig};
pub use payload::{
    DisabledOrPath, PayloadRedactionPolicy, PayloadSocketCaptureBackend, PayloadSocketConfig,
    PayloadSocketSeccompSyscall, PayloadStdioConfig, PayloadTlsCaptureBackend, PayloadTlsConfig,
    PayloadTlsLibrary, PayloadTlsLibraryPath, PayloadTlsResolver, PayloadTlsSeccompSyscall,
    PayloadTlsSource,
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
    pub payload_tls: PayloadTlsConfig,
    pub payload_stdio: PayloadStdioConfig,
    pub payload_socket: PayloadSocketConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectorConfig {
    pub ebpf: EbpfCollectorConfig,
    pub payload_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonConfig {
    pub storage_path: PathBuf,
    pub control_socket_path: PathBuf,
    pub control_socket_permissions: SocketPermissions,
    pub plugin_directories: Vec<PathBuf>,
    pub collector: CollectorConfig,
    pub retention_policy_name: String,
    pub export_policy_name: String,
}

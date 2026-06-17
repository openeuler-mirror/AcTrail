//! Command-line input shapes for the eBPF probe tool.

use std::path::PathBuf;

use config_core::daemon::{
    ApplicationProtocolConfig, EnforcementConfig, PayloadSocketConfig, PayloadStdioConfig,
    PayloadTlsConfig, ResourceMetricsConfig,
};

#[path = "args/parse.rs"]
mod parse;

pub use parse::{parse_args, workload_from_live_config};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveVerificationConfig {
    pub source_config_path: Option<PathBuf>,
    pub storage_path: PathBuf,
    pub profile_name: String,
    pub trace_name: String,
    pub request_id_start: u64,
    pub memlock_rlimit: config_core::daemon::MemlockRlimit,
    pub tracked_process_max_entries: u32,
    pub pending_operation_max_entries: u32,
    pub suppressed_fd_max_entries: u32,
    pub event_ring_buffer_max_bytes: u32,
    pub file_path_capture_enabled: bool,
    pub file_path_max_bytes: u32,
    pub payload_tls: PayloadTlsConfig,
    pub payload_stdio: PayloadStdioConfig,
    pub payload_socket: PayloadSocketConfig,
    pub application_protocol: ApplicationProtocolConfig,
    pub resource_metrics: ResourceMetricsConfig,
    pub enforcement: EnforcementConfig,
    pub process_signal_number: u32,
    pub exec_path: PathBuf,
    pub listen_addr: String,
    pub client_message: String,
    pub server_message: String,
    pub stdio_stdin_message: String,
    pub stdio_continue_message: String,
    pub stdio_stdout_message: String,
    pub stdio_stderr_message: String,
    pub pipe_message: String,
    pub fifo_path: PathBuf,
    pub fifo_mode: u32,
    pub file_path: PathBuf,
    pub file_message: String,
    pub mmap: Option<MmapWorkloadConfig>,
    pub mkdir_path: PathBuf,
    pub rmdir_path: PathBuf,
    pub rename_source_path: PathBuf,
    pub rename_target_path: PathBuf,
    pub unlink_path: PathBuf,
    pub truncate_path: PathBuf,
    pub unix_message: String,
    pub directory_mode: u32,
    pub provider_rules_path: PathBuf,
    pub provider_unknown_provider_label: String,
    pub provider_expected_provider: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmapWorkloadConfig {
    pub path: PathBuf,
    pub message: String,
    pub length: u64,
    pub offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkloadConfig {
    pub exec_path: PathBuf,
    pub listen_addr: String,
    pub client_message: String,
    pub server_message: String,
    pub stdio_stdin_message: String,
    pub stdio_continue_message: String,
    pub stdio_stdout_message: String,
    pub stdio_stderr_message: String,
    pub process_signal_number: u32,
    pub pipe_message: String,
    pub fifo_path: PathBuf,
    pub fifo_mode: u32,
    pub file_path: PathBuf,
    pub file_message: String,
    pub mmap: Option<MmapWorkloadConfig>,
    pub mkdir_path: PathBuf,
    pub rmdir_path: PathBuf,
    pub rename_source_path: PathBuf,
    pub rename_target_path: PathBuf,
    pub unlink_path: PathBuf,
    pub truncate_path: PathBuf,
    pub unix_message: String,
    pub directory_mode: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeCommand {
    VerifyLive(LiveVerificationConfig),
    Workload(WorkloadConfig),
}

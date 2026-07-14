//! Cluster reporting configuration.

use std::path::PathBuf;

pub const DEFAULT_CLUSTER_REPORT_INTERVAL_SECS: u64 = 300;
pub const DEFAULT_CLUSTER_REPORT_BATCH_MAX_TRACES: u32 = 20;
pub const DEFAULT_CLUSTER_REPORT_UPLOAD_TIMEOUT_SECS: u64 = 60;
pub const DEFAULT_CLUSTER_REPORT_RETRY_BACKOFF_SECS: u64 = 30;
pub const DEFAULT_CLUSTER_REPORT_MAX_RETRY_BACKOFF_SECS: u64 = 600;
pub const DEFAULT_CLUSTER_REPORT_BUNDLE_RETENTION_DAYS: u32 = 7;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterConfig {
    pub enabled: bool,
    pub cluster_id: String,
    pub node_id: String,
    pub node_name: String,
    pub node_ip: String,
    pub report: ClusterReportConfig,
    pub center: ClusterCenterConfig,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cluster_id: String::new(),
            node_id: String::new(),
            node_name: String::new(),
            node_ip: String::new(),
            report: ClusterReportConfig::default(),
            center: ClusterCenterConfig::default(),
        }
    }
}

impl ClusterConfig {
    pub fn trace_uid(&self, local_trace_id: &str) -> String {
        format!(
            "{}/{}/{}/{}",
            self.cluster_id, self.node_ip, self.node_id, local_trace_id
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterReportConfig {
    pub enabled: bool,
    pub center_host: String,
    pub center_port: u16,
    pub scheme: String,
    pub interval_secs: u64,
    pub terminal_only: bool,
    pub spool_dir: PathBuf,
    pub state_path: PathBuf,
    pub batch_max_traces: u32,
    pub bundle_retention_days: u32,
    pub upload_timeout_secs: u64,
    pub retry_backoff_secs: u64,
    pub max_retry_backoff_secs: u64,
    pub auth_token_file: Option<PathBuf>,
}

impl Default for ClusterReportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            center_host: String::new(),
            center_port: 0,
            scheme: "http".to_string(),
            interval_secs: DEFAULT_CLUSTER_REPORT_INTERVAL_SECS,
            terminal_only: true,
            spool_dir: PathBuf::from("/var/lib/actrail/cluster-spool"),
            state_path: PathBuf::from("/var/lib/actrail/cluster-report-state.sqlite"),
            batch_max_traces: DEFAULT_CLUSTER_REPORT_BATCH_MAX_TRACES,
            bundle_retention_days: DEFAULT_CLUSTER_REPORT_BUNDLE_RETENTION_DAYS,
            upload_timeout_secs: DEFAULT_CLUSTER_REPORT_UPLOAD_TIMEOUT_SECS,
            retry_backoff_secs: DEFAULT_CLUSTER_REPORT_RETRY_BACKOFF_SECS,
            max_retry_backoff_secs: DEFAULT_CLUSTER_REPORT_MAX_RETRY_BACKOFF_SECS,
            auth_token_file: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterCenterConfig {
    pub enabled: bool,
    pub listen_host: String,
    pub listen_port: u16,
    pub root_dir: PathBuf,
    pub auth_token_file: Option<PathBuf>,
}

impl Default for ClusterCenterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_host: String::new(),
            listen_port: 0,
            root_dir: PathBuf::from("/var/lib/actrail-cluster"),
            auth_token_file: None,
        }
    }
}

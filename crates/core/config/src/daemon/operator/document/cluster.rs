use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ClusterDocument {
    pub enabled: bool,
    pub cluster_id: String,
    pub node_id: String,
    pub node_name: String,
    pub node_ip: String,
    pub report: ClusterReportDocument,
    pub center: ClusterCenterDocument,
}

impl Default for ClusterDocument {
    fn default() -> Self {
        let config = ClusterConfig::default();
        Self {
            enabled: config.enabled,
            cluster_id: config.cluster_id,
            node_id: config.node_id,
            node_name: config.node_name,
            node_ip: config.node_ip,
            report: ClusterReportDocument::default(),
            center: ClusterCenterDocument::default(),
        }
    }
}

impl ClusterDocument {
    pub(super) fn from_config(config: &ClusterConfig) -> Self {
        Self {
            enabled: config.enabled,
            cluster_id: config.cluster_id.clone(),
            node_id: config.node_id.clone(),
            node_name: config.node_name.clone(),
            node_ip: config.node_ip.clone(),
            report: ClusterReportDocument::from_config(&config.report),
            center: ClusterCenterDocument::from_config(&config.center),
        }
    }

    pub(super) fn to_config(&self) -> Result<ClusterConfig, String> {
        if self.enabled {
            required_non_empty("cluster.cluster_id", &self.cluster_id)?;
            required_non_empty("cluster.node_id", &self.node_id)?;
            required_non_empty("cluster.node_name", &self.node_name)?;
            required_non_empty("cluster.node_ip", &self.node_ip)?;
        }
        Ok(ClusterConfig {
            enabled: self.enabled,
            cluster_id: self.cluster_id.clone(),
            node_id: self.node_id.clone(),
            node_name: self.node_name.clone(),
            node_ip: self.node_ip.clone(),
            report: self.report.to_config(self.enabled)?,
            center: self.center.to_config()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ClusterReportDocument {
    pub enabled: bool,
    pub center_host: String,
    pub center_port: u16,
    pub scheme: String,
    pub interval_secs: u64,
    pub terminal_only: bool,
    pub spool_dir: String,
    pub state_path: String,
    pub batch_max_traces: u32,
    pub bundle_retention_days: u32,
    pub upload_timeout_secs: u64,
    pub retry_backoff_secs: u64,
    pub max_retry_backoff_secs: u64,
    pub auth_token_file: String,
}

impl Default for ClusterReportDocument {
    fn default() -> Self {
        let config = ClusterReportConfig::default();
        Self {
            enabled: config.enabled,
            center_host: config.center_host,
            center_port: config.center_port,
            scheme: config.scheme,
            interval_secs: config.interval_secs,
            terminal_only: config.terminal_only,
            spool_dir: config.spool_dir.display().to_string(),
            state_path: config.state_path.display().to_string(),
            batch_max_traces: config.batch_max_traces,
            bundle_retention_days: config.bundle_retention_days,
            upload_timeout_secs: config.upload_timeout_secs,
            retry_backoff_secs: config.retry_backoff_secs,
            max_retry_backoff_secs: config.max_retry_backoff_secs,
            auth_token_file: config
                .auth_token_file
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }
}

impl ClusterReportDocument {
    pub(super) fn from_config(config: &ClusterReportConfig) -> Self {
        Self {
            enabled: config.enabled,
            center_host: config.center_host.clone(),
            center_port: config.center_port,
            scheme: config.scheme.clone(),
            interval_secs: config.interval_secs,
            terminal_only: config.terminal_only,
            spool_dir: config.spool_dir.display().to_string(),
            state_path: config.state_path.display().to_string(),
            batch_max_traces: config.batch_max_traces,
            bundle_retention_days: config.bundle_retention_days,
            upload_timeout_secs: config.upload_timeout_secs,
            retry_backoff_secs: config.retry_backoff_secs,
            max_retry_backoff_secs: config.max_retry_backoff_secs,
            auth_token_file: config
                .auth_token_file
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }

    pub(super) fn to_config(&self, cluster_enabled: bool) -> Result<ClusterReportConfig, String> {
        if cluster_enabled && self.enabled {
            required_non_empty("cluster.report.center_host", &self.center_host)?;
            if self.center_port == 0 {
                return Err(
                    "invalid cluster.report.center_port: value must be positive".to_string()
                );
            }
            if self.scheme != "http" {
                return Err(format!(
                    "invalid cluster.report.scheme: expected http, got {}",
                    self.scheme
                ));
            }
        }
        Ok(ClusterReportConfig {
            enabled: self.enabled,
            center_host: self.center_host.clone(),
            center_port: self.center_port,
            scheme: self.scheme.clone(),
            interval_secs: require_positive_u64(
                "cluster.report.interval_secs",
                self.interval_secs,
            )?,
            terminal_only: self.terminal_only,
            spool_dir: PathBuf::from(&self.spool_dir),
            state_path: PathBuf::from(&self.state_path),
            batch_max_traces: require_positive_u32(
                "cluster.report.batch_max_traces",
                self.batch_max_traces,
            )?,
            bundle_retention_days: require_positive_u32(
                "cluster.report.bundle_retention_days",
                self.bundle_retention_days,
            )?,
            upload_timeout_secs: require_positive_u64(
                "cluster.report.upload_timeout_secs",
                self.upload_timeout_secs,
            )?,
            retry_backoff_secs: require_positive_u64(
                "cluster.report.retry_backoff_secs",
                self.retry_backoff_secs,
            )?,
            max_retry_backoff_secs: require_positive_u64(
                "cluster.report.max_retry_backoff_secs",
                self.max_retry_backoff_secs,
            )?,
            auth_token_file: if self.auth_token_file.is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.auth_token_file))
            },
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ClusterCenterDocument {
    pub enabled: bool,
    pub listen_host: String,
    pub listen_port: u16,
    pub root_dir: String,
    pub auth_token_file: String,
}

impl Default for ClusterCenterDocument {
    fn default() -> Self {
        let config = ClusterCenterConfig::default();
        Self {
            enabled: config.enabled,
            listen_host: config.listen_host,
            listen_port: config.listen_port,
            root_dir: config.root_dir.display().to_string(),
            auth_token_file: config
                .auth_token_file
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }
}

impl ClusterCenterDocument {
    pub(super) fn from_config(config: &ClusterCenterConfig) -> Self {
        Self {
            enabled: config.enabled,
            listen_host: config.listen_host.clone(),
            listen_port: config.listen_port,
            root_dir: config.root_dir.display().to_string(),
            auth_token_file: config
                .auth_token_file
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }

    pub(super) fn to_config(&self) -> Result<ClusterCenterConfig, String> {
        if self.enabled {
            required_non_empty("cluster.center.listen_host", &self.listen_host)?;
            if self.listen_port == 0 {
                return Err(
                    "invalid cluster.center.listen_port: value must be positive".to_string()
                );
            }
        }
        Ok(ClusterCenterConfig {
            enabled: self.enabled,
            listen_host: self.listen_host.clone(),
            listen_port: self.listen_port,
            root_dir: PathBuf::from(&self.root_dir),
            auth_token_file: if self.auth_token_file.is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.auth_token_file))
            },
        })
    }
}

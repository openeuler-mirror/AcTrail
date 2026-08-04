use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct IpcLineageDocument {
    pub(super) enabled: bool,
    pub(super) max_processes_per_trace: u32,
    pub(super) max_candidate_fds_per_trace: u32,
    pub(super) max_stdio_bundles_per_trace: u32,
}

impl Default for IpcLineageDocument {
    fn default() -> Self {
        Self::from_config(&IpcLineageConfig::default())
    }
}

impl IpcLineageDocument {
    pub(in crate::daemon::operator::document) fn from_config(config: &IpcLineageConfig) -> Self {
        Self {
            enabled: config.enabled,
            max_processes_per_trace: config.max_processes_per_trace,
            max_candidate_fds_per_trace: config.max_candidate_fds_per_trace,
            max_stdio_bundles_per_trace: config.max_stdio_bundles_per_trace,
        }
    }

    pub(super) fn to_config(&self) -> Result<IpcLineageConfig, String> {
        Ok(IpcLineageConfig {
            enabled: self.enabled,
            max_processes_per_trace: require_positive_u32(
                "ebpf.ipc_lineage.max_processes_per_trace",
                self.max_processes_per_trace,
            )?,
            max_candidate_fds_per_trace: require_positive_u32(
                "ebpf.ipc_lineage.max_candidate_fds_per_trace",
                self.max_candidate_fds_per_trace,
            )?,
            max_stdio_bundles_per_trace: require_positive_u32(
                "ebpf.ipc_lineage.max_stdio_bundles_per_trace",
                self.max_stdio_bundles_per_trace,
            )?,
        })
    }
}

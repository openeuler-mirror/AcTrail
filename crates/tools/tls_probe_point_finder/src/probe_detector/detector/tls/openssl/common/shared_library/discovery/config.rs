use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};

pub(crate) const DEFAULT_MAX_DEPENDENCY_NODES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenSslSharedLibraryDiscoveryProbeDetectorConfig {
    pub(crate) python_ssl_query_enabled: bool,
    pub(crate) max_dependency_nodes: usize,
}

impl Default for OpenSslSharedLibraryDiscoveryProbeDetectorConfig {
    fn default() -> Self {
        Self {
            python_ssl_query_enabled: true,
            max_dependency_nodes: DEFAULT_MAX_DEPENDENCY_NODES,
        }
    }
}

impl ProbeDetectorConfig for OpenSslSharedLibraryDiscoveryProbeDetectorConfig {
    fn validate(&self) -> Result<(), DetectorConfigError> {
        if self.max_dependency_nodes == 0 {
            return Err(DetectorConfigError::new(
                "OpenSSL max_dependency_nodes must be greater than zero",
            ));
        }
        Ok(())
    }
}
